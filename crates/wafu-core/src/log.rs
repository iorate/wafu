use std::collections::VecDeque;

const MAX_LINES: usize = 1000;

/// The log shown at the bottom of the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Log {
    lines: VecDeque<String>,
    /// The first visible line when not following the tail.
    scroll_top: usize,
    /// Whether new lines keep the tail in view.
    follow: bool,
    viewport_rows: usize,
}

impl Log {
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: VecDeque::new(),
            scroll_top: 0,
            follow: true,
            viewport_rows: 1,
        }
    }

    pub fn push(&mut self, line: String) {
        if self.lines.len() == MAX_LINES {
            self.lines.pop_front();
            // Keep the same lines in view after the oldest one is dropped.
            self.scroll_top = self.scroll_top.saturating_sub(1);
        }
        self.lines.push_back(line);
    }

    pub fn lines(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().map(String::as_str)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.lines.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The first visible line.
    #[must_use]
    pub fn scroll_top(&self) -> usize {
        if self.follow {
            self.max_top()
        } else {
            self.scroll_top.min(self.max_top())
        }
    }

    pub fn set_viewport_rows(&mut self, rows: usize) {
        self.viewport_rows = rows.max(1);
    }

    pub fn scroll_up(&mut self) {
        self.scroll_by(-1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_by(1);
    }

    pub fn page_up(&mut self) {
        self.scroll_by(-i64::try_from(self.viewport_rows).unwrap_or(i64::MAX));
    }

    pub fn page_down(&mut self) {
        self.scroll_by(i64::try_from(self.viewport_rows).unwrap_or(i64::MAX));
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.scroll_top = 0;
        self.follow = true;
    }

    /// Scrolls by a number of lines; positive moves towards the tail.
    pub fn scroll_by(&mut self, delta: i64) {
        let top = i64::try_from(self.scroll_top()).unwrap_or(i64::MAX);
        let max_top = self.max_top();
        let target = usize::try_from(top.saturating_add(delta)).unwrap_or(0);
        if target >= max_top {
            self.follow = true;
        } else {
            self.follow = false;
            self.scroll_top = target;
        }
    }

    fn max_top(&self) -> usize {
        self.lines.len().saturating_sub(self.viewport_rows)
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

    #[test]
    fn push_follows_the_tail_by_default() {
        let mut log = log_with(5, 3);

        assert_eq!(log.scroll_top(), 2);
        log.push("f".to_owned());
        assert_eq!(log.scroll_top(), 3);
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
    fn scroll_up_stops_at_the_top() {
        let mut log = log_with(5, 3);

        log.scroll_up();
        log.scroll_up();
        log.scroll_up();

        assert_eq!(log.scroll_top(), 0);
    }

    #[test]
    fn page_moves_by_the_viewport() {
        let mut log = log_with(10, 3);

        log.page_up();
        assert_eq!(log.scroll_top(), 4);
        log.page_up();
        assert_eq!(log.scroll_top(), 1);
        log.page_down();
        assert_eq!(log.scroll_top(), 4);
        log.page_down();
        assert_eq!(log.scroll_top(), 7);
    }

    #[test]
    fn short_log_has_no_scroll() {
        let mut log = log_with(2, 3);

        log.scroll_up();
        log.scroll_down();

        assert_eq!(log.scroll_top(), 0);
    }

    #[test]
    fn oldest_lines_are_dropped_at_the_limit() {
        let mut log = Log::new();
        for i in 0..MAX_LINES + 2 {
            log.push(i.to_string());
        }

        assert_eq!(log.len(), MAX_LINES);
        assert_eq!(log.lines().next(), Some("2"));
    }

    #[test]
    fn dropping_a_line_keeps_the_view_in_place_when_scrolled() {
        let mut log = Log::new();
        log.set_viewport_rows(3);
        for i in 0..MAX_LINES {
            log.push(i.to_string());
        }
        log.scroll_up();
        let first_visible = log.lines().nth(log.scroll_top()).unwrap().to_owned();

        log.push("new".to_owned());

        assert_eq!(
            log.lines().nth(log.scroll_top()),
            Some(first_visible.as_str())
        );
    }

    #[test]
    fn clear_empties_and_follows() {
        let mut log = log_with(5, 3);
        log.scroll_up();

        log.clear();

        assert_eq!(log.len(), 0);
        assert_eq!(log.scroll_top(), 0);
        log.push("a".to_owned());
        log.push("b".to_owned());
        log.push("c".to_owned());
        log.push("d".to_owned());
        assert_eq!(log.scroll_top(), 1);
    }

    fn log_with(lines: usize, viewport_rows: usize) -> Log {
        let mut log = Log::new();
        log.set_viewport_rows(viewport_rows);
        for i in 0..lines {
            log.push(i.to_string());
        }
        log
    }
}
