use std::ffi::OsString;
use std::sync::Arc;

use crate::entry::Entry;
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
    /// The name to put the cursor on once the next listing arrives.
    pub pending_cursor: Option<OsString>,
    pub loading: Option<RequestId>,
    rows: Arc<[RowView]>,
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
            pending_cursor: None,
            loading: None,
            rows: Arc::from([]),
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

    /// Replaces the location and empties the listing until the new one arrives.
    pub fn set_location(&mut self, location: Location, pending_cursor: Option<OsString>) {
        self.location = location;
        self.pending_cursor = pending_cursor;
        self.listing = Listing::default();
        self.cursor = 0;
        self.scroll_top = 0;
        self.rebuild_rows();
    }

    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        self.listing = Listing::new(entries, self.sort, self.show_hidden);
        self.rebuild_rows();
        let pending = self.pending_cursor.take();
        self.cursor = pending.and_then(|name| self.row_of(&name)).unwrap_or(0);
        self.scroll_top = 0;
        self.ensure_cursor_visible();
    }

    pub fn set_viewport_rows(&mut self, rows: usize) {
        self.viewport_rows = rows.max(1);
        self.ensure_cursor_visible();
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
        let max_top = count.saturating_sub(self.viewport_rows);
        self.scroll_top = (self.scroll_top + self.viewport_rows).min(max_top);
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

    fn row_of(&self, name: &OsString) -> Option<usize> {
        let offset = usize::from(self.has_parent());
        self.listing
            .visible_entries()
            .position(|entry| entry.name == *name)
            .map(|index| index + offset)
    }

    fn rebuild_rows(&mut self) {
        let parent = self.has_parent().then(RowView::parent);
        self.rows = parent
            .into_iter()
            .chain(self.listing.visible_entries().map(RowView::from_entry))
            .collect();
    }

    fn ensure_cursor_visible(&mut self) {
        if self.cursor < self.scroll_top {
            self.scroll_top = self.cursor;
        } else if self.cursor >= self.scroll_top + self.viewport_rows {
            self.scroll_top = self.cursor + 1 - self.viewport_rows;
        }
    }
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
    fn set_entries_falls_back_to_first_row_when_pending_name_is_missing() {
        let mut pane = pane_with("/home", &["a", "b", "c"]);
        pane.cursor = 3;
        pane.pending_cursor = Some(OsString::from("missing"));

        pane.set_entries(entries(&["a", "b", "c"]));

        assert_eq!(pane.cursor, 0);
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
    fn set_location_empties_the_listing_until_entries_arrive() {
        let mut pane = pane_with("/", &["a", "b"]);
        pane.cursor_bottom();

        pane.set_location(
            Location::new(PathBuf::from("/home")),
            Some(OsString::from("b")),
        );

        assert_eq!(pane.row_count(), 1);
        assert_eq!(pane.rows().len(), 1);
        assert_eq!(pane.cursor, 0);
        assert_eq!(pane.pending_cursor, Some(OsString::from("b")));
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

    fn pane_with(base: &str, names: &[&str]) -> Pane {
        let mut pane = Pane::new(Location::new(PathBuf::from(base)));
        pane.set_entries(entries(names));
        pane
    }

    fn entries(names: &[&str]) -> Vec<Entry> {
        names
            .iter()
            .map(|name| Entry {
                name: OsString::from(name),
                kind: EntryKind::File,
                size: 0,
                modified: None,
                hidden: false,
            })
            .collect()
    }
}
