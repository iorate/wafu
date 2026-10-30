use std::path::{Path, PathBuf};

const MAX_ITEMS: usize = 100;

/// The places a pane has visited, browser style: a list and a position in it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct History {
    items: Vec<PathBuf>,
    pos: usize,
}

impl History {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Rebuilds a saved history. The position is clamped to the items and the
    /// oldest items beyond the limit are dropped.
    #[must_use]
    pub fn restore(mut items: Vec<PathBuf>, pos: usize) -> Self {
        let excess = items.len().saturating_sub(MAX_ITEMS);
        items.drain(..excess);
        let pos = pos
            .saturating_sub(excess)
            .min(items.len().saturating_sub(1));
        Self { items, pos }
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
        if self.items.len() > MAX_ITEMS {
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_history_has_nothing_to_move_to() {
        let history = History::new();

        assert_eq!(history.back(), None);
        assert_eq!(history.forward(), None);
        assert_eq!(history.items(), Vec::<PathBuf>::new());
    }

    #[test]
    fn visits_append_and_move_the_position() {
        let mut history = History::new();

        history.visit(PathBuf::from("/a"));
        history.visit(PathBuf::from("/b"));

        assert_eq!(history.items(), [PathBuf::from("/a"), PathBuf::from("/b")]);
        assert_eq!(history.pos(), 1);
        assert_eq!(history.back(), Some((0, Path::new("/a"))));
        assert_eq!(history.forward(), None);
    }

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
    fn visiting_after_going_back_drops_the_forward_side() {
        let mut history = History::new();
        history.visit(PathBuf::from("/a"));
        history.visit(PathBuf::from("/b"));
        history.visit(PathBuf::from("/c"));
        history.set_pos(0);

        history.visit(PathBuf::from("/d"));

        assert_eq!(history.items(), [PathBuf::from("/a"), PathBuf::from("/d")]);
        assert_eq!(history.pos(), 1);
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
        let history = History::restore(vec![PathBuf::from("/a"), PathBuf::from("/b")], 5);
        assert_eq!(history.pos(), 1);
        assert_eq!(History::restore(Vec::new(), 3).pos(), 0);

        let items: Vec<PathBuf> = (0..MAX_ITEMS + 2)
            .map(|i| PathBuf::from(format!("/{i}")))
            .collect();
        let history = History::restore(items, 1);
        assert_eq!(history.items().len(), MAX_ITEMS);
        assert_eq!(history.items()[0], PathBuf::from("/2"));
        assert_eq!(history.pos(), 0);
    }

    #[test]
    fn the_oldest_item_is_dropped_past_the_limit() {
        let mut history = History::new();
        for i in 0..=MAX_ITEMS {
            history.visit(PathBuf::from(format!("/{i}")));
        }

        assert_eq!(history.items().len(), MAX_ITEMS);
        assert_eq!(history.items()[0], PathBuf::from("/1"));
        assert_eq!(history.pos(), MAX_ITEMS - 1);
        assert_eq!(history.forward(), None);
    }
}
