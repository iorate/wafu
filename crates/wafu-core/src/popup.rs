use std::path::PathBuf;

use crate::command::{Command, Context};
use crate::history::History;
use crate::listing::{SortKey, SortSpec};
use crate::view::{ItemMark, PopupItem};

/// A modal that takes all keys while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    Sort(SortPopup),
    Input(InputPopup),
    Search(SearchPopup),
    Palette(PalettePopup),
    List(ListPopup),
}

/// Edits a copy of the sort settings; nothing is applied until confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortPopup {
    pub draft: SortSpec,
    /// Row in [`SortPopup::items`]: the sort keys first, then the three toggles.
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputPopup {
    pub kind: InputKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Filter,
    MarkGlob,
    GoToPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPopup {
    pub query: String,
    pub origin_cursor: usize,
    pub origin_scroll_top: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PalettePopup {
    pub context: Context,
    pub query: String,
    pub matches: Vec<Command>,
    pub cursor: usize,
    /// The names of the external commands, listed after the built-in ones.
    externals: Vec<String>,
}

/// A list of places to pick from: history, bookmarks, or drives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPopup {
    pub kind: ListKind,
    pub items: Vec<ListItem>,
    pub cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    History,
    Bookmarks,
    Drives,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    /// The key that picks the item at once, if it has one.
    pub key: Option<char>,
    pub label: String,
    /// Where the pane already is; picking it just closes the popup.
    pub current: bool,
    pub target: ListTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListTarget {
    /// A position in the pane's history.
    HistoryPos(usize),
    Path(PathBuf),
    Drive(char),
}

/// The keys given to the first 36 items of a history or bookmark list.
const ITEM_KEYS: &str = "0123456789abcdefghijklmnopqrstuvwxyz";

const SORT_KEYS: [(SortKey, &str, char); 6] = [
    (SortKey::None, "None", 'N'),
    (SortKey::Name, "Name", 'F'),
    (SortKey::Extension, "Extension", 'E'),
    (SortKey::Size, "Size", 'S'),
    (SortKey::Time, "Time", 'T'),
    (SortKey::Natural, "Name (natural)", 'X'),
];

const SORT_TOGGLES: [(&str, char); 3] = [
    ("Descending", 'D'),
    ("Directories first", 'G'),
    ("Case-insensitive", 'C'),
];

impl SortPopup {
    pub const ROWS: usize = SORT_KEYS.len() + SORT_TOGGLES.len();

    #[must_use]
    pub fn new(sort: SortSpec) -> Self {
        Self {
            draft: sort,
            cursor: SORT_KEYS
                .iter()
                .position(|(key, _, _)| *key == sort.key)
                .unwrap_or(0),
        }
    }

    #[must_use]
    pub fn items(&self) -> Vec<PopupItem> {
        let keys = SORT_KEYS.iter().map(|(key, label, letter)| PopupItem {
            label: (*label).to_owned(),
            detail: letter.to_string(),
            mark: ItemMark::Radio(self.draft.key == *key),
        });
        let toggles = SORT_TOGGLES
            .iter()
            .zip(self.toggle_values())
            .map(|((label, letter), on)| PopupItem {
                label: (*label).to_owned(),
                detail: letter.to_string(),
                mark: ItemMark::Check(on),
            });
        keys.chain(toggles).collect()
    }

    pub fn cursor_up(&mut self) {
        self.cursor = self.cursor.checked_sub(1).unwrap_or(Self::ROWS - 1);
    }

    pub fn cursor_down(&mut self) {
        self.cursor = (self.cursor + 1) % Self::ROWS;
    }

    /// Selects the sort key on the cursor row, or flips the toggle there.
    pub fn activate_cursor(&mut self) {
        if let Some((key, _, _)) = SORT_KEYS.get(self.cursor) {
            self.draft.key = *key;
        } else {
            self.toggle(self.cursor - SORT_KEYS.len());
        }
    }

    /// The sort key selected by a letter, if the letter stands for one.
    #[must_use]
    pub fn key_for_letter(letter: char) -> Option<SortKey> {
        SORT_KEYS
            .iter()
            .find(|(_, _, l)| *l == letter)
            .map(|(key, _, _)| *key)
    }

    /// Flips the toggle a letter stands for. Returns whether the letter was one.
    pub fn toggle_for_letter(&mut self, letter: char) -> bool {
        match SORT_TOGGLES.iter().position(|(_, l)| *l == letter) {
            Some(index) => {
                self.toggle(index);
                true
            }
            None => false,
        }
    }

    fn toggle_values(&self) -> [bool; 3] {
        [
            self.draft.descending,
            self.draft.dirs_first,
            self.draft.ignore_case,
        ]
    }

    fn toggle(&mut self, index: usize) {
        let flag = match index {
            0 => &mut self.draft.descending,
            1 => &mut self.draft.dirs_first,
            _ => &mut self.draft.ignore_case,
        };
        *flag = !*flag;
    }
}

impl InputKind {
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Filter => "Filter",
            Self::MarkGlob => "Mark",
            Self::GoToPath => "Go to path",
        }
    }
}

impl ListPopup {
    #[must_use]
    pub fn new(kind: ListKind, items: Vec<ListItem>) -> Self {
        let cursor = items.iter().position(|item| item.current).unwrap_or(0);
        Self {
            kind,
            items,
            cursor,
        }
    }

    /// The history, newest first.
    #[must_use]
    pub fn history(history: &History) -> Self {
        let count = history.items().len();
        let items = history
            .items()
            .iter()
            .enumerate()
            .rev()
            .zip(item_keys())
            .map(|((pos, path), key)| ListItem {
                key,
                label: path.display().to_string(),
                current: count > 0 && pos == history.pos(),
                target: ListTarget::HistoryPos(pos),
            })
            .collect();
        Self::new(ListKind::History, items)
    }

    /// Bookmarks in the configured order, already expanded.
    #[must_use]
    pub fn bookmarks(paths: Vec<PathBuf>) -> Self {
        let items = paths
            .into_iter()
            .zip(item_keys())
            .map(|(path, key)| ListItem {
                key,
                label: path.display().to_string(),
                current: false,
                target: ListTarget::Path(path),
            })
            .collect();
        Self::new(ListKind::Bookmarks, items)
    }

    /// Drive letters in order, each picked by its own letter.
    #[must_use]
    pub fn drives(letters: &[char], current: Option<char>) -> Self {
        let items = letters
            .iter()
            .map(|&letter| ListItem {
                key: Some(letter.to_ascii_lowercase()),
                label: format!("{letter}:\\"),
                current: Some(letter) == current,
                target: ListTarget::Drive(letter),
            })
            .collect();
        Self::new(ListKind::Drives, items)
    }

    #[must_use]
    pub fn title(&self) -> &'static str {
        match self.kind {
            ListKind::History => "History",
            ListKind::Bookmarks => "Bookmarks",
            ListKind::Drives => "Drives",
        }
    }

    /// What to show instead of an empty list.
    #[must_use]
    pub fn empty_message(&self) -> &'static str {
        match self.kind {
            ListKind::History => "No history",
            ListKind::Bookmarks => "No bookmarks",
            ListKind::Drives => "No drives",
        }
    }

    /// The rows to show: the place, with its key at the end like the sort popup.
    #[must_use]
    pub fn items(&self) -> Vec<PopupItem> {
        self.items
            .iter()
            .map(|item| PopupItem {
                label: item.label.clone(),
                detail: item
                    .key
                    .map(|key| key.to_ascii_uppercase().to_string())
                    .unwrap_or_default(),
                mark: ItemMark::None,
            })
            .collect()
    }

    #[must_use]
    pub fn selected(&self) -> Option<&ListItem> {
        self.items.get(self.cursor)
    }

    /// The item a key picks at once.
    #[must_use]
    pub fn item_for_key(&self, key: char) -> Option<&ListItem> {
        let key = key.to_ascii_lowercase();
        self.items.iter().find(|item| item.key == Some(key))
    }

    pub fn cursor_up(&mut self) {
        if !self.items.is_empty() {
            self.cursor = self.cursor.checked_sub(1).unwrap_or(self.items.len() - 1);
        }
    }

    pub fn cursor_down(&mut self) {
        if !self.items.is_empty() {
            self.cursor = (self.cursor + 1) % self.items.len();
        }
    }
}

fn item_keys() -> impl Iterator<Item = Option<char>> {
    ITEM_KEYS.chars().map(Some).chain(std::iter::repeat(None))
}

impl PalettePopup {
    #[must_use]
    pub fn new(context: Context, externals: Vec<String>) -> Self {
        Self {
            context,
            query: String::new(),
            matches: matches(context, &externals, ""),
            cursor: 0,
            externals,
        }
    }

    pub fn set_query(&mut self, query: String) {
        self.matches = matches(self.context, &self.externals, &query);
        self.query = query;
        self.cursor = 0;
    }

    #[must_use]
    pub fn selected(&self) -> Option<Command> {
        self.matches.get(self.cursor).cloned()
    }

    pub fn cursor_up(&mut self) {
        if !self.matches.is_empty() {
            self.cursor = self.cursor.checked_sub(1).unwrap_or(self.matches.len() - 1);
        }
    }

    pub fn cursor_down(&mut self) {
        if !self.matches.is_empty() {
            self.cursor = (self.cursor + 1) % self.matches.len();
        }
    }
}

fn matches(context: Context, externals: &[String], query: &str) -> Vec<Command> {
    let query = query.to_lowercase();
    Command::all()
        .chain(externals.iter().map(|name| Command::External(name.clone())))
        .filter(|command| command.available_in(context))
        .filter(|command| command.to_string().contains(&query))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_popup_opens_on_the_current_key() {
        let popup = SortPopup::new(SortSpec {
            key: SortKey::Size,
            ..SortSpec::default()
        });

        assert_eq!(popup.cursor, 3);
        let items = popup.items();
        let rows: Vec<(&str, ItemMark)> =
            items.iter().map(|i| (i.label.as_str(), i.mark)).collect();
        assert_eq!(
            rows,
            [
                ("None", ItemMark::Radio(false)),
                ("Name", ItemMark::Radio(false)),
                ("Extension", ItemMark::Radio(false)),
                ("Size", ItemMark::Radio(true)),
                ("Time", ItemMark::Radio(false)),
                ("Name (natural)", ItemMark::Radio(false)),
                ("Descending", ItemMark::Check(false)),
                ("Directories first", ItemMark::Check(true)),
                ("Case-insensitive", ItemMark::Check(true)),
            ]
        );
        assert_eq!(items[5].detail, "X");
    }

    #[test]
    fn sort_popup_cursor_wraps() {
        let mut popup = SortPopup::new(SortSpec::default());
        popup.cursor = 0;

        popup.cursor_up();
        assert_eq!(popup.cursor, SortPopup::ROWS - 1);
        popup.cursor_down();
        assert_eq!(popup.cursor, 0);
    }

    #[test]
    fn sort_popup_activate_selects_keys_and_flips_toggles() {
        let mut popup = SortPopup::new(SortSpec::default());

        popup.cursor = 4;
        popup.activate_cursor();
        assert_eq!(popup.draft.key, SortKey::Time);

        popup.cursor = 6;
        popup.activate_cursor();
        assert!(popup.draft.descending);
        popup.activate_cursor();
        assert!(!popup.draft.descending);
    }

    #[test]
    fn sort_popup_letters() {
        let mut popup = SortPopup::new(SortSpec::default());

        assert_eq!(SortPopup::key_for_letter('X'), Some(SortKey::Natural));
        assert_eq!(SortPopup::key_for_letter('D'), None);
        assert!(popup.toggle_for_letter('C'));
        assert!(!popup.draft.ignore_case);
        assert!(popup.toggle_for_letter('G'));
        assert!(!popup.draft.dirs_first);
        assert!(!popup.toggle_for_letter('Q'));
    }

    #[test]
    fn palette_filters_by_substring_ignoring_case() {
        let mut popup = PalettePopup::new(Context::FileList, Vec::new());
        assert_eq!(popup.matches.len(), Command::all().count());

        popup.set_query("MARK_R".to_owned());

        assert_eq!(
            popup.matches,
            [Command::MarkRangeAbove, Command::MarkRangeBelow]
        );
        assert_eq!(popup.selected(), Some(Command::MarkRangeAbove));
    }

    #[test]
    fn palette_shows_only_commands_available_in_the_context() {
        let popup = PalettePopup::new(Context::Image, vec!["x".to_owned()]);

        assert_eq!(popup.matches.len(), 16);
        assert!(!popup.matches.contains(&Command::Copy));
    }

    #[test]
    fn palette_lists_external_commands_after_the_built_in_ones() {
        let mut popup = PalettePopup::new(Context::FileList, vec!["open_terminal".to_owned()]);

        assert_eq!(
            popup.matches.last(),
            Some(&Command::External("open_terminal".to_owned()))
        );
        popup.set_query("$open".to_owned());
        assert_eq!(
            popup.matches,
            [Command::External("open_terminal".to_owned())]
        );
    }

    #[test]
    fn list_popup_gives_keys_to_the_first_36_items_only() {
        let paths = (0..40).map(|i| PathBuf::from(format!("/{i}"))).collect();

        let popup = ListPopup::bookmarks(paths);

        assert_eq!(popup.items[0].key, Some('0'));
        assert_eq!(popup.items[9].key, Some('9'));
        assert_eq!(popup.items[10].key, Some('a'));
        assert_eq!(popup.items[35].key, Some('z'));
        assert_eq!(popup.items[36].key, None);
        assert_eq!(popup.items()[35].detail, "Z");
        assert_eq!(popup.items()[36].detail, "");
        assert_eq!(
            popup.item_for_key('Z').map(|item| item.label.as_str()),
            Some("/35")
        );
        assert_eq!(popup.item_for_key('!'), None);
    }

    #[test]
    fn list_popup_of_history_is_newest_first_with_the_cursor_on_the_current_place() {
        let mut history = History::new();
        history.visit(PathBuf::from("/a"));
        history.visit(PathBuf::from("/b"));
        history.visit(PathBuf::from("/c"));
        history.set_pos(1);

        let popup = ListPopup::history(&history);

        let labels: Vec<&str> = popup.items.iter().map(|item| item.label.as_str()).collect();
        assert_eq!(labels, ["/c", "/b", "/a"]);
        assert_eq!(popup.items[1].target, ListTarget::HistoryPos(1));
        assert!(popup.items[1].current);
        assert_eq!(popup.cursor, 1);
    }

    #[test]
    fn list_popup_of_drives_uses_the_letters_as_keys() {
        let popup = ListPopup::drives(&['C', 'D'], Some('D'));

        assert_eq!(popup.items[0].key, Some('c'));
        assert_eq!(popup.items[0].target, ListTarget::Drive('C'));
        assert_eq!(popup.items()[1].label, "D:\\");
        assert_eq!(popup.items()[1].detail, "D");
        assert_eq!(popup.cursor, 1);
        assert_eq!(popup.empty_message(), "No drives");
    }

    #[test]
    fn list_popup_cursor_wraps_and_survives_an_empty_list() {
        let mut popup = ListPopup::bookmarks(vec![PathBuf::from("/a"), PathBuf::from("/b")]);
        popup.cursor_up();
        assert_eq!(popup.cursor, 1);
        popup.cursor_down();
        assert_eq!(popup.cursor, 0);

        let mut empty = ListPopup::bookmarks(Vec::new());
        empty.cursor_down();
        assert_eq!(empty.selected(), None);
        assert_eq!(empty.empty_message(), "No bookmarks");
    }

    #[test]
    fn palette_cursor_wraps_and_handles_no_matches() {
        let mut popup = PalettePopup::new(Context::FileList, Vec::new());
        popup.set_query("jump_".to_owned());
        assert_eq!(popup.matches.len(), 36);

        popup.cursor_up();
        assert_eq!(popup.cursor, 35);
        popup.cursor_down();
        assert_eq!(popup.cursor, 0);

        popup.set_query("nothing matches this".to_owned());
        popup.cursor_down();
        assert_eq!(popup.selected(), None);
    }
}
