use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::sync::Arc;

use crate::entry::Entry;
use crate::filter::Filter;
use crate::listing::{Listing, SortSpec};
use crate::location::Location;
use crate::view::RowView;

/// A unique number for an asynchronous request, used to discard stale replies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RequestId(pub u64);

/// One of the two panes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pane {
    pub location: Location,
    pub listing: Listing,
    /// Index into the rows, where row 0 is `..` when the location has a parent.
    pub cursor: usize,
    pub scroll_top: usize,
    pub viewport_rows: usize,
    pub sort: SortSpec,
    pub show_hidden: bool,
    pub filter: Option<Filter>,
    /// Marked names. Kept by name so that marks survive a reload.
    pub marks: HashSet<OsString>,
    /// The name to put the cursor on once the next listing arrives.
    pub pending_cursor: Option<OsString>,
    pub loading: Option<RequestId>,
    /// A change was reported while a read was in flight; read again when it finishes.
    pub reload_pending: bool,
    rows: Arc<[RowView]>,
    mark_flags: Arc<[bool]>,
}

/// How a command picks the entries it works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetRule {
    /// The entry under the cursor, ignoring marks.
    Single,
    /// The marked entries, or the entry under the cursor when nothing is marked.
    Multiple,
}

impl Pane {
    #[must_use]
    pub fn new(location: Location) -> Self {
        Self {
            location,
            listing: Listing::default(),
            cursor: 0,
            scroll_top: 0,
            viewport_rows: 1,
            sort: SortSpec::default(),
            show_hidden: false,
            filter: None,
            marks: HashSet::new(),
            pending_cursor: None,
            loading: None,
            reload_pending: false,
            rows: Arc::from([]),
            mark_flags: Arc::from([]),
        }
    }

    #[must_use]
    pub fn has_parent(&self) -> bool {
        self.location.parent().is_some()
    }

    #[must_use]
    pub fn row_count(&self) -> usize {
        usize::from(self.has_parent()) + self.listing.visible_len()
    }

    /// The entry on a row, or `None` for the `..` row.
    #[must_use]
    pub fn entry_at(&self, row: usize) -> Option<&Entry> {
        if self.has_parent() {
            row.checked_sub(1)
                .and_then(|index| self.listing.visible_entry(index))
        } else {
            self.listing.visible_entry(row)
        }
    }

    #[must_use]
    pub fn cursor_entry(&self) -> Option<&Entry> {
        self.entry_at(self.cursor)
    }

    /// The rows to display, including `..`. Rebuilt only when the listing changes,
    /// so that rendering after a cursor move costs nothing per row.
    #[must_use]
    pub fn rows(&self) -> Arc<[RowView]> {
        Arc::clone(&self.rows)
    }

    /// Whether each row is marked. Rebuilt only when the marks or the rows change.
    #[must_use]
    pub fn mark_flags(&self) -> Arc<[bool]> {
        Arc::clone(&self.mark_flags)
    }

    #[must_use]
    pub fn is_marked(&self, entry: &Entry) -> bool {
        self.marks.contains(&entry.name)
    }

    /// The marked entries in display order.
    pub fn marked_entries(&self) -> impl Iterator<Item = &Entry> {
        self.listing
            .visible_entries()
            .filter(|entry| self.is_marked(entry))
    }

    /// The number of marked entries and their total size, excluding directories.
    #[must_use]
    pub fn marked_summary(&self) -> (usize, u64) {
        self.marked_entries().fold((0, 0), |(count, size), entry| {
            let own = if entry.kind.is_dir_like() {
                0
            } else {
                entry.size
            };
            (count + 1, size + own)
        })
    }

    #[must_use]
    pub fn targets(&self, rule: TargetRule) -> Vec<&Entry> {
        if rule == TargetRule::Multiple && !self.marks.is_empty() {
            return self.marked_entries().collect();
        }
        self.cursor_entry().into_iter().collect()
    }

    /// Replaces the location and empties the listing until the new one arrives.
    /// Marks and the filter belong to the old location and are dropped.
    pub fn set_location(&mut self, location: Location, pending_cursor: Option<OsString>) {
        self.location = location;
        self.pending_cursor = pending_cursor;
        self.listing = Listing::default();
        self.cursor = 0;
        self.scroll_top = 0;
        self.marks.clear();
        self.filter = None;
        self.reload_pending = false;
        self.rebuild_rows();
    }

    /// Installs a freshly read listing. The cursor follows the pending name if
    /// any, else the entry it was on, else it stays on the same row.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        let follow = self
            .pending_cursor
            .take()
            .or_else(|| self.cursor_entry().map(|entry| entry.name.clone()));
        self.listing = Listing::new(entries, self.sort, self.show_hidden, self.filter.as_ref());
        self.after_listing_change(follow);
    }

    pub fn set_sort(&mut self, sort: SortSpec) {
        self.sort = sort;
        self.refresh_view();
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.marks.clear();
        self.refresh_view();
    }

    /// Returns `false` when the filter is unchanged, in which case nothing happens.
    pub fn set_filter(&mut self, filter: Option<Filter>) -> bool {
        if self.filter == filter {
            return false;
        }
        self.filter = filter;
        self.marks.clear();
        self.refresh_view();
        true
    }

    pub fn set_viewport_rows(&mut self, rows: usize) {
        self.viewport_rows = rows.max(1);
        self.ensure_cursor_visible();
    }

    pub fn set_cursor(&mut self, row: usize) {
        if row < self.row_count() {
            self.cursor = row;
            self.ensure_cursor_visible();
        }
    }

    pub fn cursor_up(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        self.cursor = if self.cursor == 0 {
            count - 1
        } else {
            self.cursor - 1
        };
        self.ensure_cursor_visible();
    }

    pub fn cursor_down(&mut self) {
        let count = self.row_count();
        if count == 0 {
            return;
        }
        self.cursor = if self.cursor + 1 >= count {
            0
        } else {
            self.cursor + 1
        };
        self.ensure_cursor_visible();
    }

    pub fn page_up(&mut self) {
        self.scroll_top = self.scroll_top.saturating_sub(self.viewport_rows);
        self.cursor = self.cursor.saturating_sub(self.viewport_rows);
        self.ensure_cursor_visible();
    }

    pub fn page_down(&mut self) {
        let count = self.row_count();
        self.scroll_top = (self.scroll_top + self.viewport_rows).min(self.max_scroll_top());
        self.cursor = (self.cursor + self.viewport_rows).min(count.saturating_sub(1));
        self.ensure_cursor_visible();
    }

    pub fn cursor_top(&mut self) {
        self.cursor = 0;
        self.ensure_cursor_visible();
    }

    pub fn cursor_bottom(&mut self) {
        self.cursor = self.row_count().saturating_sub(1);
        self.ensure_cursor_visible();
    }

    /// Scrolls the view without moving the cursor.
    pub fn scroll_by(&mut self, rows: i32) {
        let top = i64::try_from(self.scroll_top).unwrap_or(i64::MAX) + i64::from(rows);
        let max = i64::try_from(self.max_scroll_top()).unwrap_or(i64::MAX);
        self.scroll_top = usize::try_from(top.clamp(0, max)).unwrap_or(0);
    }

    pub fn ensure_cursor_visible(&mut self) {
        if self.cursor < self.scroll_top {
            self.scroll_top = self.cursor;
        } else if self.cursor >= self.scroll_top + self.viewport_rows {
            self.scroll_top = self.cursor + 1 - self.viewport_rows;
        }
    }

    /// Toggles the mark on the cursor row and moves down, stopping at the end.
    pub fn mark_down(&mut self) {
        self.toggle_mark_at(self.cursor);
        if self.cursor + 1 < self.row_count() {
            self.cursor += 1;
        }
        self.ensure_cursor_visible();
    }

    /// Toggles the mark on the cursor row and moves up, stopping at the top.
    pub fn mark_up(&mut self) {
        self.toggle_mark_at(self.cursor);
        self.cursor = self.cursor.saturating_sub(1);
        self.ensure_cursor_visible();
    }

    /// Marks from the nearest mark above the cursor down to the cursor.
    pub fn mark_range_above(&mut self) {
        let Some(start) = (0..self.cursor).rev().find(|&row| self.row_is_marked(row)) else {
            return;
        };
        self.mark_rows(start..=self.cursor);
    }

    /// Marks from the cursor down to the nearest mark below it.
    pub fn mark_range_below(&mut self) {
        let Some(end) = (self.cursor + 1..self.row_count()).find(|&row| self.row_is_marked(row))
        else {
            return;
        };
        self.mark_rows(self.cursor..=end);
    }

    pub fn invert_marks(&mut self, include_dirs: bool) {
        for entry in self.listing.visible_entries() {
            if (include_dirs || !entry.kind.is_dir_like()) && !self.marks.remove(&entry.name) {
                self.marks.insert(entry.name.clone());
            }
        }
        self.rebuild_mark_flags();
    }

    /// Adds marks on the visible files matching the filter.
    pub fn mark_matching(&mut self, filter: &Filter) {
        for entry in self.listing.visible_entries() {
            if !entry.kind.is_dir_like() && filter.matches(&entry.name) {
                self.marks.insert(entry.name.clone());
            }
        }
        self.rebuild_mark_flags();
    }

    /// Moves to the next marked row, wrapping around.
    pub fn next_mark(&mut self) {
        if let Some(row) = self.find_row_from(self.cursor + 1, Self::row_is_marked) {
            self.cursor = row;
            self.ensure_cursor_visible();
        }
    }

    /// Moves to the previous marked row, wrapping around.
    pub fn prev_mark(&mut self) {
        let start = self
            .cursor
            .checked_sub(1)
            .unwrap_or(self.row_count().saturating_sub(1));
        if let Some(row) = self.find_row_backward_from(start, Self::row_is_marked) {
            self.cursor = row;
            self.ensure_cursor_visible();
        }
    }

    pub fn clear_marks(&mut self) {
        self.marks.clear();
        self.rebuild_mark_flags();
    }

    /// Moves to the next row whose name starts with the character, wrapping around.
    pub fn jump_to(&mut self, c: char) {
        let prefix = c.to_lowercase().to_string();
        if let Some(row) = self.find_row_from(self.cursor + 1, |pane, row| {
            pane.row_starts_with(row, &prefix)
        }) {
            self.cursor = row;
            self.ensure_cursor_visible();
        }
    }

    /// Moves to the first row whose name starts with the prefix. Returns whether one exists.
    pub fn search_first(&mut self, prefix: &str) -> bool {
        self.search_from(0, prefix, true)
    }

    pub fn search_next(&mut self, prefix: &str) -> bool {
        self.search_from(self.cursor + 1, prefix, true)
    }

    pub fn search_prev(&mut self, prefix: &str) -> bool {
        let start = self
            .cursor
            .checked_sub(1)
            .unwrap_or(self.row_count().saturating_sub(1));
        self.search_from(start, prefix, false)
    }

    fn search_from(&mut self, start: usize, prefix: &str, forward: bool) -> bool {
        let prefix = prefix.to_lowercase();
        let found = if forward {
            self.find_row_from(start, |pane, row| pane.row_starts_with(row, &prefix))
        } else {
            self.find_row_backward_from(start, |pane, row| pane.row_starts_with(row, &prefix))
        };
        if let Some(row) = found {
            self.cursor = row;
            self.ensure_cursor_visible();
        }
        found.is_some()
    }

    fn find_row_from(&self, start: usize, pred: impl Fn(&Self, usize) -> bool) -> Option<usize> {
        let count = self.row_count();
        if count == 0 {
            return None;
        }
        (0..count)
            .map(|offset| (start + offset) % count)
            .find(|&row| pred(self, row))
    }

    fn find_row_backward_from(
        &self,
        start: usize,
        pred: impl Fn(&Self, usize) -> bool,
    ) -> Option<usize> {
        let count = self.row_count();
        if count == 0 {
            return None;
        }
        (0..count)
            .map(|offset| (start + count - offset % count) % count)
            .find(|&row| pred(self, row))
    }

    fn row_is_marked(&self, row: usize) -> bool {
        self.entry_at(row)
            .is_some_and(|entry| self.is_marked(entry))
    }

    fn row_starts_with(&self, row: usize, lower_prefix: &str) -> bool {
        self.entry_at(row)
            .is_some_and(|entry| name_starts_with(&entry.name, lower_prefix))
    }

    fn toggle_mark_at(&mut self, row: usize) {
        let Some(name) = self.entry_at(row).map(|entry| entry.name.clone()) else {
            return;
        };
        if !self.marks.remove(&name) {
            self.marks.insert(name);
        }
        self.rebuild_mark_flags();
    }

    fn mark_rows(&mut self, rows: std::ops::RangeInclusive<usize>) {
        for row in rows {
            if let Some(entry) = self.entry_at(row) {
                self.marks.insert(entry.name.clone());
            }
        }
        self.rebuild_mark_flags();
    }

    fn refresh_view(&mut self) {
        let follow = self.cursor_entry().map(|entry| entry.name.clone());
        self.listing
            .rebuild(self.sort, self.show_hidden, self.filter.as_ref());
        self.after_listing_change(follow);
    }

    fn after_listing_change(&mut self, follow: Option<OsString>) {
        self.prune_marks();
        self.rebuild_rows();
        self.cursor = follow
            .and_then(|name| self.row_of(&name))
            .unwrap_or_else(|| self.cursor.min(self.row_count().saturating_sub(1)));
        self.scroll_top = self.scroll_top.min(self.max_scroll_top());
        self.ensure_cursor_visible();
    }

    fn prune_marks(&mut self) {
        if self.marks.is_empty() {
            return;
        }
        let visible: HashSet<&OsStr> = self
            .listing
            .visible_entries()
            .map(|entry| entry.name.as_os_str())
            .collect();
        self.marks.retain(|name| visible.contains(name.as_os_str()));
    }

    fn row_of(&self, name: &OsString) -> Option<usize> {
        let offset = usize::from(self.has_parent());
        self.listing
            .visible_entries()
            .position(|entry| entry.name == *name)
            .map(|index| index + offset)
    }

    fn max_scroll_top(&self) -> usize {
        self.row_count().saturating_sub(self.viewport_rows)
    }

    fn rebuild_rows(&mut self) {
        let parent = self.has_parent().then(RowView::parent);
        self.rows = parent
            .into_iter()
            .chain(self.listing.visible_entries().map(RowView::from_entry))
            .collect();
        self.rebuild_mark_flags();
    }

    fn rebuild_mark_flags(&mut self) {
        let parent = self.has_parent().then_some(false);
        self.mark_flags = parent
            .into_iter()
            .chain(
                self.listing
                    .visible_entries()
                    .map(|entry| self.is_marked(entry)),
            )
            .collect();
    }
}

fn name_starts_with(name: &OsStr, lower_prefix: &str) -> bool {
    name.to_string_lossy()
        .to_lowercase()
        .starts_with(lower_prefix)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::entry::EntryKind;

    #[test]
    fn rows_start_with_parent_when_location_has_parent() {
        let pane = pane_with("/home", &["a", "b"]);

        assert_eq!(pane.row_count(), 3);
        assert_eq!(pane.entry_at(0), None);
        assert_eq!(
            pane.entry_at(1).map(|e| &e.name),
            Some(&OsString::from("a"))
        );
    }

    #[test]
    fn rows_have_no_parent_at_root() {
        let pane = pane_with("/", &["a", "b"]);

        assert_eq!(pane.row_count(), 2);
        assert_eq!(
            pane.entry_at(0).map(|e| &e.name),
            Some(&OsString::from("a"))
        );
    }

    #[test]
    fn set_entries_puts_cursor_on_pending_name() {
        let mut pane = Pane::new(Location::new(PathBuf::from("/home")));
        pane.pending_cursor = Some(OsString::from("b"));

        pane.set_entries(entries(&["a", "b", "c"]));

        assert_eq!(pane.cursor, 2);
        assert_eq!(pane.pending_cursor, None);
    }

    #[test]
    fn set_entries_after_navigation_falls_back_to_first_row() {
        let mut pane = pane_with("/home", &["a", "b", "c"]);
        pane.cursor_bottom();
        pane.set_location(
            Location::new(PathBuf::from("/other")),
            Some(OsString::from("missing")),
        );

        pane.set_entries(entries(&["a", "b", "c"]));

        assert_eq!(pane.cursor, 0);
    }

    #[test]
    fn set_entries_on_reload_follows_the_cursor_entry() {
        let mut pane = pane_with("/", &["a", "b", "c"]);
        pane.cursor = 1;

        pane.set_entries(entries(&["b", "c", "d"]));

        assert_eq!(pane.cursor, 0);
        assert_eq!(cursor_name(&pane), "b");
    }

    #[test]
    fn set_entries_on_reload_keeps_the_row_when_the_entry_is_gone() {
        let mut pane = pane_with("/", &["a", "b", "c", "d"]);
        pane.cursor = 1;

        pane.set_entries(entries(&["a", "c", "d"]));
        assert_eq!(pane.cursor, 1);
        assert_eq!(cursor_name(&pane), "c");

        pane.cursor = 2;
        pane.set_entries(entries(&["a"]));
        assert_eq!(pane.cursor, 0);
    }

    #[test]
    fn set_entries_on_reload_keeps_the_scroll_and_marks_of_remaining_entries() {
        let mut pane = pane_with("/", &["a", "b", "c", "d", "e"]);
        pane.set_viewport_rows(2);
        pane.cursor = 3;
        pane.ensure_cursor_visible();
        pane.mark_down();
        pane.mark_down();
        assert_eq!(pane.scroll_top, 3);

        pane.set_entries(entries(&["a", "b", "c", "e", "f"]));

        assert_eq!(pane.scroll_top, 3);
        assert_eq!(marked(&pane), ["e"]);
    }

    #[test]
    fn rows_are_shared_until_the_listing_changes() {
        let mut pane = pane_with("/home", &["a", "b"]);
        let before = pane.rows();

        pane.cursor_down();
        assert!(Arc::ptr_eq(&before, &pane.rows()));
        assert_eq!(before.len(), 3);
        assert_eq!(before[0].name, "..");

        pane.set_entries(entries(&["c"]));
        assert!(!Arc::ptr_eq(&before, &pane.rows()));
        assert_eq!(pane.rows().len(), 2);
    }

    #[test]
    fn mark_flags_are_shared_until_the_marks_change() {
        let mut pane = pane_with("/home", &["a", "b"]);
        let before = pane.mark_flags();

        pane.cursor_down();
        assert!(Arc::ptr_eq(&before, &pane.mark_flags()));

        pane.mark_down();
        let after = pane.mark_flags();
        assert!(!Arc::ptr_eq(&before, &after));
        assert_eq!(after.as_ref(), [false, true, false]);
    }

    #[test]
    fn set_location_empties_the_listing_and_drops_marks_and_filter() {
        let mut pane = pane_with("/", &["a", "b"]);
        pane.mark_down();
        pane.set_filter(Filter::parse("*").unwrap());

        pane.set_location(
            Location::new(PathBuf::from("/home")),
            Some(OsString::from("b")),
        );

        assert_eq!(pane.row_count(), 1);
        assert_eq!(pane.rows().len(), 1);
        assert_eq!(pane.cursor, 0);
        assert_eq!(pane.pending_cursor, Some(OsString::from("b")));
        assert_eq!(pane.marks, HashSet::new());
        assert_eq!(pane.filter, None);
    }

    #[test]
    fn cursor_up_wraps_to_last_row() {
        let mut pane = pane_with("/", &["a", "b", "c"]);

        pane.cursor_up();

        assert_eq!(pane.cursor, 2);
    }

    #[test]
    fn cursor_down_wraps_to_first_row() {
        let mut pane = pane_with("/", &["a", "b", "c"]);
        pane.cursor = 2;

        pane.cursor_down();

        assert_eq!(pane.cursor, 0);
    }

    #[test]
    fn cursor_moves_do_nothing_on_empty_listing() {
        let mut pane = pane_with("/", &[]);

        pane.cursor_up();
        pane.cursor_down();
        pane.page_up();
        pane.page_down();
        pane.cursor_top();
        pane.cursor_bottom();
        pane.mark_down();
        pane.mark_up();
        pane.next_mark();
        pane.prev_mark();
        pane.jump_to('a');

        assert_eq!(pane.cursor, 0);
        assert_eq!(pane.scroll_top, 0);
    }

    #[test]
    fn page_down_stops_at_last_row() {
        let mut pane = pane_with("/", &["a", "b", "c", "d", "e"]);
        pane.set_viewport_rows(3);

        pane.page_down();
        assert_eq!(pane.cursor, 3);
        pane.page_down();
        assert_eq!(pane.cursor, 4);
        pane.page_down();
        assert_eq!(pane.cursor, 4);
    }

    #[test]
    fn page_up_stops_at_first_row() {
        let mut pane = pane_with("/", &["a", "b", "c", "d", "e"]);
        pane.set_viewport_rows(3);
        pane.cursor_bottom();

        pane.page_up();
        assert_eq!(pane.cursor, 1);
        pane.page_up();
        assert_eq!(pane.cursor, 0);
        pane.page_up();
        assert_eq!(pane.cursor, 0);
    }

    #[test]
    fn page_down_and_up_move_the_view_by_a_page_and_keep_the_cursor_position() {
        let mut pane = pane_with("/", &["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"]);
        pane.set_viewport_rows(4);
        pane.cursor_down();

        pane.page_down();
        assert_eq!((pane.scroll_top, pane.cursor), (4, 5));
        pane.page_down();
        assert_eq!((pane.scroll_top, pane.cursor), (6, 9));
        pane.page_up();
        assert_eq!((pane.scroll_top, pane.cursor), (2, 5));
        pane.page_up();
        assert_eq!((pane.scroll_top, pane.cursor), (0, 1));
    }

    #[test]
    fn scroll_follows_cursor_down_and_up() {
        let mut pane = pane_with("/", &["a", "b", "c", "d", "e"]);
        pane.set_viewport_rows(2);

        pane.cursor_down();
        assert_eq!(pane.scroll_top, 0);
        pane.cursor_down();
        assert_eq!(pane.scroll_top, 1);
        pane.cursor_bottom();
        assert_eq!(pane.scroll_top, 3);
        pane.cursor_top();
        assert_eq!(pane.scroll_top, 0);
    }

    #[test]
    fn shrinking_viewport_keeps_cursor_visible() {
        let mut pane = pane_with("/", &["a", "b", "c", "d", "e"]);
        pane.set_viewport_rows(5);
        pane.cursor_bottom();

        pane.set_viewport_rows(2);

        assert_eq!(pane.scroll_top, 3);
    }

    #[test]
    fn scroll_by_moves_the_view_within_bounds_without_moving_the_cursor() {
        let mut pane = pane_with("/", &["a", "b", "c", "d", "e"]);
        pane.set_viewport_rows(2);

        pane.scroll_by(2);
        assert_eq!((pane.scroll_top, pane.cursor), (2, 0));
        pane.scroll_by(10);
        assert_eq!(pane.scroll_top, 3);
        pane.scroll_by(-10);
        assert_eq!(pane.scroll_top, 0);
    }

    #[test]
    fn set_cursor_ignores_rows_out_of_range() {
        let mut pane = pane_with("/", &["a", "b", "c"]);

        pane.set_cursor(2);
        assert_eq!(pane.cursor, 2);
        pane.set_cursor(3);
        assert_eq!(pane.cursor, 2);
    }

    #[test]
    fn mark_down_toggles_and_stops_at_the_end() {
        let mut pane = pane_with("/", &["a", "b"]);

        pane.mark_down();
        assert_eq!(marked(&pane), ["a"]);
        assert_eq!(pane.cursor, 1);
        pane.mark_down();
        assert_eq!(marked(&pane), ["a", "b"]);
        assert_eq!(pane.cursor, 1);
        pane.mark_down();
        assert_eq!(marked(&pane), ["a"]);
        assert_eq!(pane.cursor, 1);
    }

    #[test]
    fn mark_up_toggles_and_stops_at_the_top() {
        let mut pane = pane_with("/", &["a", "b"]);
        pane.cursor = 1;

        pane.mark_up();
        assert_eq!(marked(&pane), ["b"]);
        assert_eq!(pane.cursor, 0);
        pane.mark_up();
        assert_eq!(marked(&pane), ["a", "b"]);
        assert_eq!(pane.cursor, 0);
    }

    #[test]
    fn parent_row_cannot_be_marked() {
        let mut pane = pane_with("/home", &["a"]);

        pane.mark_down();

        assert_eq!(pane.marks, HashSet::new());
        assert_eq!(pane.cursor, 1);
    }

    #[test]
    fn mark_range_above_marks_from_the_nearest_mark_above() {
        let mut pane = pane_with("/home", &["a", "b", "c", "d", "e"]);
        pane.cursor = 1;
        pane.mark_down();
        pane.cursor = 4;

        pane.mark_range_above();

        assert_eq!(marked(&pane), ["a", "b", "c", "d"]);
        assert_eq!(pane.cursor, 4);
    }

    #[test]
    fn mark_range_above_does_nothing_without_a_mark_above() {
        let mut pane = pane_with("/home", &["a", "b", "c"]);
        pane.cursor = 3;
        pane.mark_up();
        pane.cursor = 1;

        pane.mark_range_above();

        assert_eq!(marked(&pane), ["c"]);
    }

    #[test]
    fn mark_range_below_marks_down_to_the_nearest_mark_below() {
        let mut pane = pane_with("/home", &["a", "b", "c", "d", "e"]);
        pane.cursor = 4;
        pane.mark_up();
        pane.cursor = 0;

        pane.mark_range_below();
        assert_eq!(marked(&pane), ["a", "b", "c", "d"]);

        pane.marks.clear();
        pane.cursor = 2;
        pane.mark_range_below();
        assert_eq!(pane.marks, HashSet::new());
    }

    #[test]
    fn invert_marks_skips_directories_unless_asked() {
        let mut pane = Pane::new(Location::new(PathBuf::from("/")));
        pane.set_entries(vec![dir("d"), file("a"), file("b")]);
        pane.cursor = 1;
        pane.mark_down();

        pane.invert_marks(false);
        assert_eq!(marked(&pane), ["b"]);

        pane.invert_marks(true);
        assert_eq!(marked(&pane), ["d", "a"]);
    }

    #[test]
    fn mark_matching_adds_files_only() {
        let mut pane = Pane::new(Location::new(PathBuf::from("/")));
        pane.set_entries(vec![dir("d.png"), file("a.png"), file("b.txt")]);
        pane.cursor = 2;
        pane.mark_down();

        pane.mark_matching(&Filter::parse("*.png").unwrap().unwrap());

        assert_eq!(marked(&pane), ["a.png", "b.txt"]);
    }

    #[test]
    fn next_mark_wraps_around() {
        let mut pane = pane_with("/home", &["a", "b", "c"]);
        pane.cursor = 1;
        pane.mark_down();
        pane.cursor = 3;
        pane.mark_down();
        pane.cursor = 2;

        pane.next_mark();
        assert_eq!(pane.cursor, 3);
        pane.next_mark();
        assert_eq!(pane.cursor, 1);
    }

    #[test]
    fn prev_mark_wraps_around() {
        let mut pane = pane_with("/home", &["a", "b", "c"]);
        pane.cursor = 1;
        pane.mark_down();
        pane.cursor = 3;
        pane.mark_down();
        pane.cursor = 2;

        pane.prev_mark();
        assert_eq!(pane.cursor, 1);
        pane.prev_mark();
        assert_eq!(pane.cursor, 3);
    }

    #[test]
    fn next_mark_without_marks_does_nothing() {
        let mut pane = pane_with("/home", &["a", "b"]);
        pane.cursor = 1;

        pane.next_mark();

        assert_eq!(pane.cursor, 1);
    }

    #[test]
    fn marked_summary_counts_directories_without_their_size() {
        let mut pane = Pane::new(Location::new(PathBuf::from("/")));
        pane.set_entries(vec![
            Entry {
                size: 4096,
                ..dir("d")
            },
            Entry {
                size: 10,
                ..file("a")
            },
            Entry {
                size: 5,
                ..file("b")
            },
        ]);
        pane.mark_down();
        pane.mark_down();

        assert_eq!(pane.marked_summary(), (2, 10));
    }

    #[test]
    fn targets_follow_the_rule() {
        let mut pane = pane_with("/home", &["a", "b", "c"]);
        assert_eq!(pane.targets(TargetRule::Single), Vec::<&Entry>::new());
        assert_eq!(pane.targets(TargetRule::Multiple), Vec::<&Entry>::new());

        pane.cursor = 1;
        assert_eq!(names(pane.targets(TargetRule::Single)), ["a"]);
        assert_eq!(names(pane.targets(TargetRule::Multiple)), ["a"]);

        pane.cursor = 3;
        pane.mark_up();
        pane.mark_up();
        pane.cursor = 1;
        assert_eq!(names(pane.targets(TargetRule::Single)), ["a"]);
        assert_eq!(names(pane.targets(TargetRule::Multiple)), ["b", "c"]);
    }

    #[test]
    fn jump_to_finds_the_next_match_case_insensitively_and_wraps() {
        let mut pane = pane_with("/home", &["Apple", "banana", "avocado"]);

        pane.jump_to('a');
        assert_eq!(cursor_name(&pane), "Apple");
        pane.jump_to('a');
        assert_eq!(cursor_name(&pane), "avocado");
        pane.jump_to('a');
        assert_eq!(cursor_name(&pane), "Apple");
        pane.jump_to('z');
        assert_eq!(cursor_name(&pane), "Apple");
    }

    #[test]
    fn search_moves_between_matches() {
        let mut pane = pane_with("/home", &["ab", "ac", "b", "AD"]);

        assert!(pane.search_first("A"));
        assert_eq!(cursor_name(&pane), "ab");
        assert!(pane.search_next("a"));
        assert_eq!(cursor_name(&pane), "ac");
        assert!(pane.search_next("a"));
        assert_eq!(cursor_name(&pane), "AD");
        assert!(pane.search_next("a"));
        assert_eq!(cursor_name(&pane), "ab");
        assert!(pane.search_prev("a"));
        assert_eq!(cursor_name(&pane), "AD");
        assert!(!pane.search_first("x"));
        assert_eq!(cursor_name(&pane), "AD");
    }

    #[test]
    fn set_sort_keeps_marks_and_follows_the_cursor_entry() {
        let mut pane = pane_with("/", &["a", "b", "c"]);
        pane.cursor = 1;
        pane.mark_down();

        pane.set_sort(SortSpec {
            descending: true,
            ..SortSpec::default()
        });

        assert_eq!(
            names(pane.listing.visible_entries().collect()),
            ["c", "b", "a"]
        );
        assert_eq!(cursor_name(&pane), "c");
        assert_eq!(marked(&pane), ["b"]);
    }

    #[test]
    fn toggle_hidden_clears_marks() {
        let mut pane = Pane::new(Location::new(PathBuf::from("/")));
        pane.set_entries(vec![
            file("a"),
            Entry {
                hidden: true,
                ..file(".b")
            },
        ]);
        pane.mark_down();

        pane.toggle_hidden();

        assert!(pane.show_hidden);
        assert_eq!(pane.row_count(), 2);
        assert_eq!(pane.marks, HashSet::new());
    }

    #[test]
    fn set_filter_clears_marks_only_when_the_pattern_changes() {
        let mut pane = pane_with("/", &["a.png", "b.txt"]);
        pane.mark_down();

        assert!(pane.set_filter(Filter::parse("*.png").unwrap()));
        assert_eq!(pane.row_count(), 1);
        assert_eq!(pane.marks, HashSet::new());

        pane.mark_down();
        assert!(!pane.set_filter(Filter::parse("*.png").unwrap()));
        assert_eq!(marked(&pane), ["a.png"]);

        assert!(pane.set_filter(None));
        assert_eq!(pane.row_count(), 2);
        assert_eq!(pane.marks, HashSet::new());
        assert!(!pane.set_filter(None));
    }

    fn pane_with(base: &str, names: &[&str]) -> Pane {
        let mut pane = Pane::new(Location::new(PathBuf::from(base)));
        pane.set_entries(entries(names));
        pane
    }

    fn entries(names: &[&str]) -> Vec<Entry> {
        names.iter().map(|name| file(name)).collect()
    }

    fn file(name: &str) -> Entry {
        Entry {
            name: OsString::from(name),
            kind: EntryKind::File,
            size: 0,
            modified: None,
            hidden: false,
        }
    }

    fn dir(name: &str) -> Entry {
        Entry {
            kind: EntryKind::Dir,
            ..file(name)
        }
    }

    fn names(entries: Vec<&Entry>) -> Vec<String> {
        entries
            .into_iter()
            .map(|entry| entry.name.to_string_lossy().into_owned())
            .collect()
    }

    fn marked(pane: &Pane) -> Vec<String> {
        names(pane.marked_entries().collect())
    }

    fn cursor_name(pane: &Pane) -> String {
        pane.cursor_entry().map_or_else(
            || "..".to_owned(),
            |e| e.name.to_string_lossy().into_owned(),
        )
    }
}
