use std::path::{Path, PathBuf};

/// The places a pane has visited, browser style: a list and a position in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    items: Vec<PathBuf>,
    pos: usize,
    max_items: usize,
}

impl History {
    /// An empty history without a limit.
    #[must_use]
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            pos: 0,
            max_items: usize::MAX,
        }
    }

    /// Rebuilds a saved history. The position is clamped to the items and the
    /// oldest items beyond `max_items` are dropped.
    #[must_use]
    pub fn restore(items: Vec<PathBuf>, pos: usize, max_items: usize) -> Self {
        let pos = pos.min(items.len().saturating_sub(1));
        let mut history = Self {
            items,
            pos,
            max_items,
        };
        history.drop_excess();
        history
    }

    /// Sets how many places are kept. The oldest
    /// places beyond it are dropped at once, and the position moves with the
    /// places.
    pub fn set_max_items(&mut self, max_items: usize) {
        self.max_items = max_items;
        self.drop_excess();
    }

    #[must_use]
    pub fn items(&self) -> &[PathBuf] {
        &self.items
    }

    /// The current position, meaningful only when the history is not empty.
    #[must_use]
    pub fn pos(&self) -> usize {
        self.pos
    }

    /// The previous place and its position, if any.
    #[must_use]
    pub fn back(&self) -> Option<(usize, &Path)> {
        let pos = self.pos.checked_sub(1)?;
        Some((pos, self.items[pos].as_path()))
    }

    /// The next place and its position, if any.
    #[must_use]
    pub fn forward(&self) -> Option<(usize, &Path)> {
        let pos = self.pos + 1;
        Some((pos, self.items.get(pos)?.as_path()))
    }

    /// Records a new visit after the current position, dropping anything after
    /// it. Visiting the current place again records nothing.
    pub fn visit(&mut self, path: PathBuf) {
        if let Some(current) = self.items.get(self.pos)
            && *current == path
        {
            return;
        }
        self.items.truncate(self.pos + 1);
        self.items.push(path);
        if self.items.len() > self.max_items {
            self.items.remove(0);
        }
        self.pos = self.items.len() - 1;
    }

    /// Moves to a recorded place without adding a visit.
    pub fn set_pos(&mut self, pos: usize) {
        if pos < self.items.len() {
            self.pos = pos;
        }
    }

    fn drop_excess(&mut self) {
        let excess = self.items.len().saturating_sub(self.max_items);
        self.items.drain(..excess);
        self.pos = self.pos.saturating_sub(excess);
    }
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visiting_the_current_place_again_records_nothing() {
        let mut history = History::new();

        history.visit(PathBuf::from("/a"));
        history.visit(PathBuf::from("/a"));

        assert_eq!(history.items(), [PathBuf::from("/a")]);
    }

    #[test]
    fn the_same_place_may_appear_twice_when_not_consecutive() {
        let mut history = History::new();

        history.visit(PathBuf::from("/a"));
        history.visit(PathBuf::from("/b"));
        history.visit(PathBuf::from("/a"));

        assert_eq!(history.items().len(), 3);
    }

    #[test]
    fn back_returns_the_previous_place_and_stops_at_the_oldest() {
        let mut history = History::new();
        history.visit(PathBuf::from("/a"));
        history.visit(PathBuf::from("/b"));

        assert_eq!(history.back(), Some((0, Path::new("/a"))));
        history.set_pos(0);
        assert_eq!(history.back(), None);
    }

    #[test]
    fn a_visit_after_going_back_drops_the_forward_side() {
        let mut history = History::new();
        for path in ["/a", "/b", "/c"] {
            history.visit(PathBuf::from(path));
        }
        history.set_pos(0);

        history.visit(PathBuf::from("/d"));

        assert_eq!(history.items(), [PathBuf::from("/a"), PathBuf::from("/d")]);
        assert_eq!(history.pos(), 1);
    }

    #[test]
    fn visiting_the_current_place_after_going_back_keeps_the_forward_side() {
        let mut history = History::new();
        history.visit(PathBuf::from("/a"));
        history.visit(PathBuf::from("/b"));
        history.set_pos(0);

        history.visit(PathBuf::from("/a"));

        assert_eq!(history.items(), [PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(history.pos(), 0);
    }

    #[test]
    fn set_pos_moves_without_recording_and_ignores_out_of_range() {
        let mut history = History::new();
        history.visit(PathBuf::from("/a"));
        history.visit(PathBuf::from("/b"));

        history.set_pos(0);
        assert_eq!(history.pos(), 0);
        assert_eq!(history.forward(), Some((1, Path::new("/b"))));

        history.set_pos(5);
        assert_eq!(history.pos(), 0);
        assert_eq!(history.items().len(), 2);
    }

    #[test]
    fn restore_clamps_the_position_and_the_length() {
        let history = History::restore(
            vec![PathBuf::from("/a"), PathBuf::from("/b")],
            5,
            usize::MAX,
        );
        assert_eq!(history.pos(), 1);
        assert_eq!(History::restore(Vec::new(), 3, usize::MAX).pos(), 0);

        let history = History::restore(places(4), 1, 2);
        assert_eq!(history.items(), [PathBuf::from("/2"), PathBuf::from("/3")]);
        assert_eq!(history.pos(), 0);
    }

    #[test]
    fn lowering_the_limit_drops_the_oldest_and_shifts_the_position() {
        let mut history = History::restore(places(5), 3, usize::MAX);

        history.set_max_items(3);
        assert_eq!(history.items(), &places(5)[2..]);
        assert_eq!(history.pos(), 1);
    }

    #[test]
    fn the_oldest_item_is_dropped_past_the_limit() {
        let mut history = History::new();
        history.set_max_items(2);
        for path in places(3) {
            history.visit(path);
        }

        assert_eq!(history.items(), &places(3)[1..]);
        assert_eq!(history.pos(), 1);
        assert_eq!(history.forward(), None);
    }

    fn places(count: usize) -> Vec<PathBuf> {
        (0..count).map(|i| PathBuf::from(format!("/{i}"))).collect()
    }
}
