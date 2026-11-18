use std::sync::Arc;

use crate::scroll::{self, LineRows};
use crate::text::show_line_breaks;
use crate::view::LineRanges;

/// The log shown at the bottom of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Log {
    lines: Arc<[Arc<str>]>,
    rows: LineRows,
    top: usize,
    // Whether the top stays at the end as lines come and sizes change.
    follow: bool,
    viewport_rows: usize,
    generation: usize,
    max_lines: usize,
}

impl Log {
    /// An empty log without a limit.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: Arc::from([]),
            rows: LineRows::default(),
            top: 0,
            follow: true,
            viewport_rows: 1,
            generation: 0,
            max_lines: usize::MAX,
        }
    }

    /// Sets how many lines are kept, `usize::MAX` for no limit. The oldest
    /// lines beyond it are dropped at once.
    pub fn set_max_lines(&mut self, max_lines: usize) {
        self.max_lines = max_lines;
        let excess = self.lines.len().saturating_sub(max_lines);
        if excess == 0 {
            return;
        }
        self.lines = Arc::from(&self.lines[excess..]);
        for _ in 0..excess {
            self.rows.drop_first();
        }
        self.top = self.top.saturating_sub(excess);
        self.generation += 1;
        self.settle();
    }

    pub fn push(&mut self, line: String) {
        let mut kept = &self.lines[..];
        if kept.len() == self.max_lines {
            kept = &kept[1..];
            self.rows.drop_first();
            self.top = self.top.saturating_sub(1);
            self.generation += 1;
        }
        self.lines = kept
            .iter()
            .cloned()
            .chain([Arc::from(show_line_breaks(line))])
            .collect();
        self.settle();
    }

    /// Takes how many rows lines take, by index, as the UI measured them.
    /// Indices past the last line are ignored.
    pub fn set_line_rows(&mut self, reports: &[(usize, usize)]) {
        for &(index, rows) in reports {
            if index < self.lines.len() {
                self.rows.set(index, rows);
            }
        }
        self.settle();
    }

    /// The lines in order. The same allocation is returned until the lines
    /// change.
    #[must_use]
    pub fn lines(&self) -> Arc<[Arc<str>]> {
        Arc::clone(&self.lines)
    }

    /// Changes whenever the lines are renumbered or cleared.
    #[must_use]
    pub fn generation(&self) -> usize {
        self.generation
    }

    /// The line at the top of the area.
    #[must_use]
    pub fn scroll_top(&self) -> usize {
        self.top
    }

    /// The furthest the top can go.
    #[must_use]
    pub fn end(&self) -> usize {
        scroll::end(self.lines.len(), self.viewport_rows, |index| {
            self.rows.get(index)
        })
    }

    #[must_use]
    pub fn ranges(&self) -> LineRanges {
        scroll::ranges(self.top, self.lines.len(), self.viewport_rows)
    }

    #[must_use]
    pub fn viewport_rows(&self) -> usize {
        self.viewport_rows
    }

    pub fn set_viewport_rows(&mut self, rows: usize) {
        self.viewport_rows = rows.max(1);
        self.settle();
    }

    pub fn scroll_up(&mut self) {
        self.scroll_by(-1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_by(1);
    }

    pub fn page_up(&mut self) {
        let top = scroll::page_up(self.top, self.viewport_rows, |index| self.rows.get(index));
        self.move_to(top);
    }

    pub fn page_down(&mut self) {
        let top = scroll::page_down(self.top, self.lines.len(), self.viewport_rows, |index| {
            self.rows.get(index)
        });
        self.move_to(top);
    }

    pub fn top(&mut self) {
        self.move_to(0);
    }

    pub fn bottom(&mut self) {
        self.move_to(self.end());
    }

    pub fn clear(&mut self) {
        self.lines = Arc::from([]);
        self.rows.clear();
        self.top = 0;
        self.follow = true;
        self.generation += 1;
    }

    /// Scrolls by a number of lines; positive moves towards the tail.
    pub fn scroll_by(&mut self, delta: i64) {
        let top = scroll::move_by(
            self.top,
            delta,
            self.lines.len(),
            self.viewport_rows,
            |index| self.rows.get(index),
        );
        self.move_to(top);
    }

    /// Puts `top` at the top of the area, no further than the end.
    pub fn scroll_to(&mut self, top: usize) {
        self.move_to(top.min(self.end()));
    }

    fn move_to(&mut self, top: usize) {
        self.top = top;
        self.follow = top == self.end();
    }

    fn settle(&mut self) {
        let end = self.end();
        if self.follow {
            self.top = end;
            return;
        }
        // An end found with unknown lines taken as one row lies before the
        // real one: reaching it is not reaching the end.
        if self.top >= end && (end..self.lines.len()).all(|index| self.rows.get(index).is_some()) {
            self.top = end;
            self.follow = true;
        }
    }
}

impl Default for Log {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX_LINES: usize = 30;

    #[test]
    fn lowering_the_limit_drops_the_oldest_lines_with_their_rows() {
        let mut log = log_with(10, 3);
        log.set_line_rows(&[(9, 3)]);
        assert_eq!(log.scroll_top(), 9);
        let before = log.generation();

        log.set_max_lines(5);

        assert_eq!(
            log.lines().iter().map(|line| &**line).collect::<Vec<_>>(),
            ["5", "6", "7", "8", "9"]
        );
        assert_eq!(log.scroll_top(), 4);
        assert_ne!(log.generation(), before);
    }

    #[test]
    fn lowering_the_limit_keeps_the_view_in_place_when_scrolled() {
        let mut log = log_with(10, 3);
        log.scroll_to(4);

        log.set_max_lines(8);
        assert_eq!(&*log.lines()[log.scroll_top()], "4");

        log.set_max_lines(1);
        assert_eq!(log.scroll_top(), 0);
    }

    #[test]
    fn following_keeps_the_end_in_view_as_lines_wrap() {
        let mut log = log_with(5, 3);
        assert_eq!(log.scroll_top(), 2);

        log.set_line_rows(&[(4, 2)]);

        assert_eq!(log.scroll_top(), 3);
    }

    #[test]
    fn rows_reported_past_the_last_line_are_ignored() {
        let mut log = log_with(5, 3);

        log.set_line_rows(&[(7, 5)]);
        for i in 5..8 {
            log.push(i.to_string());
        }

        assert_eq!(log.scroll_top(), 5);
    }

    #[test]
    fn scrolling_up_stops_following_and_scrolling_to_the_bottom_resumes() {
        let mut log = log_with(5, 3);

        log.scroll_up();
        assert_eq!(log.scroll_top(), 1);
        log.push("f".to_owned());
        assert_eq!(log.scroll_top(), 1);

        log.scroll_down();
        log.scroll_down();
        assert_eq!(log.scroll_top(), 3);
        log.push("g".to_owned());
        assert_eq!(log.scroll_top(), 4);
    }

    #[test]
    fn a_line_unwrapping_to_bring_the_end_to_the_top_resumes_following() {
        let mut log = log_with(5, 3);
        log.set_line_rows(&[(3, 1), (4, 3)]);
        log.scroll_up();
        assert_eq!(log.scroll_top(), 3);

        log.set_line_rows(&[(4, 2)]);
        log.push("f".to_owned());

        assert_eq!(log.scroll_top(), 4);
    }

    #[test]
    fn growing_the_viewport_to_show_the_tail_resumes_following() {
        let mut log = log_with(10, 3);
        log.set_line_rows(&(0..10).map(|index| (index, 1)).collect::<Vec<_>>());
        log.scroll_up();
        log.scroll_up();

        log.set_viewport_rows(6);
        assert_eq!(log.scroll_top(), 4);
        log.push("k".to_owned());

        assert_eq!(log.scroll_top(), 5);
    }

    #[test]
    fn the_top_stays_while_the_lines_after_it_are_not_measured() {
        let mut log = log_with(20, 3);
        log.scroll_to(10);

        log.set_viewport_rows(10);
        log.push("u".to_owned());

        assert_eq!(log.scroll_top(), 10);
    }

    #[test]
    fn a_viewport_of_zero_rows_counts_as_one() {
        let mut log = log_with(5, 3);

        log.set_viewport_rows(0);

        assert_eq!(log.viewport_rows(), 1);
        assert_eq!(log.scroll_top(), 4);
    }

    #[test]
    fn scrolling_to_past_the_end_stops_at_the_end_and_follows() {
        let mut log = log_with(5, 3);
        log.scroll_to(0);

        log.scroll_to(100);
        log.push("f".to_owned());

        assert_eq!(log.scroll_top(), 3);
    }

    #[test]
    fn moving_to_the_top_stops_following_and_moving_to_the_bottom_resumes() {
        let mut log = log_with(5, 3);

        log.top();
        log.push("f".to_owned());
        assert_eq!(log.scroll_top(), 0);

        log.bottom();
        assert_eq!(log.scroll_top(), 3);
        log.push("g".to_owned());
        assert_eq!(log.scroll_top(), 4);
    }

    #[test]
    fn oldest_lines_are_dropped_at_the_limit() {
        let mut log = Log::new();
        log.set_max_lines(MAX_LINES);
        for i in 0..MAX_LINES + 2 {
            log.push(i.to_string());
        }

        assert_eq!(log.lines().len(), MAX_LINES);
        assert_eq!(&*log.lines()[0], "2");
    }

    #[test]
    fn dropping_the_oldest_line_moves_the_measured_rows_with_the_lines() {
        let mut log = log_with(MAX_LINES, 3);
        log.set_line_rows(&[(MAX_LINES - 2, 3)]);
        assert_eq!(log.scroll_top(), MAX_LINES - 1);

        log.push("new".to_owned());

        assert_eq!(log.scroll_top(), MAX_LINES - 2);
    }

    #[test]
    fn the_generation_advances_when_lines_are_renumbered_or_cleared() {
        let mut log = log_with(MAX_LINES - 1, 3);
        let first = log.generation();

        log.push("last".to_owned());
        assert_eq!(log.generation(), first);
        log.push("dropping".to_owned());
        let dropped = log.generation();
        assert_ne!(dropped, first);
        log.clear();
        assert_ne!(log.generation(), dropped);
    }

    #[test]
    fn lines_are_shared_until_they_change() {
        let mut log = log_with(2, 3);
        let before = log.lines();

        log.scroll_up();
        assert!(Arc::ptr_eq(&before, &log.lines()));

        log.push("c".to_owned());
        assert!(!Arc::ptr_eq(&before, &log.lines()));
        assert_eq!(
            log.lines().iter().map(|line| &**line).collect::<Vec<_>>(),
            ["0", "1", "c"]
        );
    }

    #[test]
    fn dropping_a_line_keeps_the_view_in_place_when_scrolled() {
        let mut log = Log::new();
        log.set_max_lines(MAX_LINES);
        log.set_viewport_rows(3);
        for i in 0..MAX_LINES {
            log.push(i.to_string());
        }
        log.scroll_up();
        let first_visible = log.lines()[log.scroll_top()].clone();

        log.push("new".to_owned());

        assert_eq!(log.lines()[log.scroll_top()], first_visible);
    }

    #[test]
    fn clear_empties_forgets_the_rows_and_follows() {
        let mut log = log_with(5, 3);
        log.set_line_rows(&[(1, 3)]);
        log.scroll_up();

        log.clear();

        assert_eq!(log.lines().len(), 0);
        assert_eq!(log.scroll_top(), 0);
        for line in ["a", "b", "c", "d"] {
            log.push(line.to_owned());
        }
        assert_eq!(log.scroll_top(), 1);
    }

    fn log_with(lines: usize, viewport_rows: usize) -> Log {
        let mut log = Log::new();
        log.set_max_lines(MAX_LINES);
        log.set_viewport_rows(viewport_rows);
        for i in 0..lines {
            log.push(i.to_string());
        }
        log
    }
}
