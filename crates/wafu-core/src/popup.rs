use std::ffi::OsString;
use std::path::PathBuf;

use crate::command::{Command, Context};
use crate::history::History;
use crate::listing::{SortKey, SortSpec};
use crate::view::{InputField, ItemMark, PopupItem};

/// A modal that takes all keys while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    Sort(SortPopup),
    Input(InputPopup),
    Search(SearchPopup),
    Palette(PalettePopup),
    List(ListPopup),
    Choice(ChoicePopup),
    Conflict(ConflictPopup),
}

/// Edits a copy of the sort settings; nothing is applied until confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortPopup {
    pub draft: SortSpec,
    /// Row in [`SortPopup::items`]: the sort keys first, then the three toggles.
    pub cursor: usize,
}

/// One or more input fields. Which field has the focus is the UI's business;
/// the core only keeps their texts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputPopup {
    pub kind: InputKind,
    pub fields: Vec<InputField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    Filter,
    MarkGlob,
    GoToPath,
    Rename {
        original: OsString,
    },
    Mkdir,
    NewFile,
    NewText,
    /// The "Rename" choice of a conflict; Esc goes back to the conflict popup.
    ConflictRename {
        source: PathBuf,
        dest: PathBuf,
        all: bool,
    },
    /// An encrypted entry was met. `retry` means the last password was wrong.
    ArchivePassword {
        archive: PathBuf,
        retry: bool,
        purpose: PasswordPurpose,
    },
    /// The name of the archive to make and its password, for the entries named.
    Compress {
        names: Vec<OsString>,
    },
}

/// Who asked for the password, and so who gets it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordPurpose {
    Operation,
    Preview,
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

/// A question with a few fixed answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChoicePopup {
    pub kind: ChoiceKind,
    pub message: String,
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceKind {
    /// The entries to delete, taken when the question was asked.
    DeleteConfirm { names: Vec<OsString> },
    /// Asked while an operation runs. The popup that was open, if any, comes
    /// back when the operation is left running.
    QuitDuringOperation { resume: Option<Box<Popup>> },
}

/// One answer of a [`ChoicePopup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub label: &'static str,
    /// The letter that picks the answer at once.
    pub key: Option<char>,
}

/// The destination of a copy or move exists; the worker waits for the answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictPopup {
    pub source: PathBuf,
    pub dest: PathBuf,
    /// "Apply to all": the answer also covers the conflicts that follow.
    pub all: bool,
    pub cursor: usize,
}

/// A row of the conflict popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictRow {
    Overwrite,
    Skip,
    Rename,
    ApplyToAll,
    Abort,
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

impl InputPopup {
    /// A popup with one field.
    #[must_use]
    pub fn single(kind: InputKind, text: String, select: Option<(usize, usize)>) -> Self {
        Self {
            kind,
            fields: vec![InputField {
                select,
                ..InputField::new(text)
            }],
        }
    }

    /// Asks for the password of an archive.
    #[must_use]
    pub fn password(archive: PathBuf, retry: bool, purpose: PasswordPurpose) -> Self {
        Self {
            kind: InputKind::ArchivePassword {
                archive,
                retry,
                purpose,
            },
            fields: vec![InputField {
                secret: true,
                ..InputField::new(String::new())
            }],
        }
    }

    /// Asks for the name of a new archive, with the part before the extension
    /// selected, and an optional password.
    #[must_use]
    pub fn compress(names: Vec<OsString>, initial_name: String) -> Self {
        let select = stem_selection(&initial_name);
        Self {
            kind: InputKind::Compress { names },
            fields: vec![
                InputField {
                    label: "Name",
                    select,
                    ..InputField::new(initial_name)
                },
                InputField {
                    label: "Password",
                    secret: true,
                    ..InputField::new(String::new())
                },
            ],
        }
    }

    /// The text of a field; empty when there is no such field.
    #[must_use]
    pub fn text(&self, field: usize) -> &str {
        self.fields
            .get(field)
            .map_or("", |field| field.text.as_str())
    }

    pub fn set_text(&mut self, field: usize, text: String) {
        if let Some(field) = self.fields.get_mut(field) {
            field.text = text;
        }
    }

    /// What to show under the title, if anything.
    #[must_use]
    pub fn message(&self) -> Option<String> {
        match &self.kind {
            InputKind::ArchivePassword { archive, retry, .. } => {
                let name = archive.file_name().unwrap_or_default().to_string_lossy();
                Some(if *retry {
                    format!("Wrong password for {name}")
                } else {
                    name.into_owned()
                })
            }
            InputKind::Compress { .. } => {
                Some("Leave the password empty for no encryption".to_owned())
            }
            _ => None,
        }
    }
}

impl InputKind {
    #[must_use]
    pub fn title(&self) -> &'static str {
        match self {
            Self::Filter => "Filter",
            Self::MarkGlob => "Mark",
            Self::GoToPath => "Go to path",
            Self::Rename { .. } | Self::ConflictRename { .. } => "Rename",
            Self::Mkdir => "Make directory",
            Self::NewFile => "New file",
            Self::NewText => "New text",
            Self::ArchivePassword { .. } => "Password",
            Self::Compress { .. } => "Compress",
        }
    }
}

/// The part of a file name to select when editing it: everything but the
/// extension, or nothing special when there is none.
#[must_use]
pub fn stem_selection(name: &str) -> Option<(usize, usize)> {
    name.char_indices()
        .skip(1)
        .filter(|&(_, c)| c == '.')
        .map(|(index, _)| index)
        .last()
        .map(|index| (0, index))
}

const DELETE_CHOICES: [Choice; 2] = [
    Choice {
        label: "Delete",
        key: Some('Y'),
    },
    Choice {
        label: "Cancel",
        key: Some('N'),
    },
];

const QUIT_CHOICES: [Choice; 2] = [
    Choice {
        label: "Continue the operation",
        key: None,
    },
    Choice {
        label: "Cancel the operation and quit",
        key: None,
    },
];

impl ChoicePopup {
    /// Asks before deleting; the first answer deletes.
    #[must_use]
    pub fn delete_confirm(names: Vec<OsString>) -> Self {
        let message = match names.as_slice() {
            [name] => format!("Delete \"{}\"?", name.to_string_lossy()),
            _ => format!("Delete {} items?", names.len()),
        };
        Self {
            kind: ChoiceKind::DeleteConfirm { names },
            message,
            cursor: 0,
        }
    }

    /// Asks whether to quit while an operation runs; the first answer keeps it running.
    #[must_use]
    pub fn quit_during_operation(resume: Option<Popup>) -> Self {
        Self {
            kind: ChoiceKind::QuitDuringOperation {
                resume: resume.map(Box::new),
            },
            message: "An operation is in progress".to_owned(),
            cursor: 0,
        }
    }

    #[must_use]
    pub fn title(&self) -> &'static str {
        match self.kind {
            ChoiceKind::DeleteConfirm { .. } => "Delete",
            ChoiceKind::QuitDuringOperation { .. } => "Quit",
        }
    }

    #[must_use]
    pub fn choices(&self) -> &'static [Choice] {
        match self.kind {
            ChoiceKind::DeleteConfirm { .. } => &DELETE_CHOICES,
            ChoiceKind::QuitDuringOperation { .. } => &QUIT_CHOICES,
        }
    }

    #[must_use]
    pub fn items(&self) -> Vec<PopupItem> {
        self.choices()
            .iter()
            .map(|choice| PopupItem {
                label: choice.label.to_owned(),
                detail: choice.key.map(String::from).unwrap_or_default(),
                mark: ItemMark::None,
            })
            .collect()
    }

    /// The answer a letter picks, if any. Case does not matter.
    #[must_use]
    pub fn choice_for_letter(&self, letter: char) -> Option<usize> {
        let letter = letter.to_ascii_uppercase();
        self.choices()
            .iter()
            .position(|choice| choice.key == Some(letter))
    }

    pub fn cursor_up(&mut self) {
        self.cursor = self
            .cursor
            .checked_sub(1)
            .unwrap_or(self.choices().len() - 1);
    }

    pub fn cursor_down(&mut self) {
        self.cursor = (self.cursor + 1) % self.choices().len();
    }
}

const CONFLICT_ROWS: [(ConflictRow, &str, &str); 5] = [
    (ConflictRow::Overwrite, "Overwrite", "O"),
    (ConflictRow::Skip, "Skip", "S"),
    (ConflictRow::Rename, "Rename", "R"),
    (ConflictRow::ApplyToAll, "Apply to all", "A"),
    (ConflictRow::Abort, "Abort", "Esc"),
];

impl ConflictPopup {
    #[must_use]
    pub fn new(source: PathBuf, dest: PathBuf, all: bool) -> Self {
        Self {
            source,
            dest,
            all,
            cursor: 0,
        }
    }

    #[must_use]
    pub fn items(&self) -> Vec<PopupItem> {
        CONFLICT_ROWS
            .iter()
            .map(|(row, label, key)| PopupItem {
                label: (*label).to_owned(),
                detail: (*key).to_owned(),
                mark: if *row == ConflictRow::ApplyToAll {
                    ItemMark::Check(self.all)
                } else {
                    ItemMark::None
                },
            })
            .collect()
    }

    #[must_use]
    pub fn row_at_cursor(&self) -> ConflictRow {
        CONFLICT_ROWS[self.cursor].0
    }

    /// The row a letter stands for. Case does not matter.
    #[must_use]
    pub fn row_for_letter(letter: char) -> Option<ConflictRow> {
        match letter.to_ascii_uppercase() {
            'O' => Some(ConflictRow::Overwrite),
            'S' => Some(ConflictRow::Skip),
            'R' => Some(ConflictRow::Rename),
            'A' => Some(ConflictRow::ApplyToAll),
            _ => None,
        }
    }

    pub fn cursor_up(&mut self) {
        self.cursor = self
            .cursor
            .checked_sub(1)
            .unwrap_or(CONFLICT_ROWS.len() - 1);
    }

    pub fn cursor_down(&mut self) {
        self.cursor = (self.cursor + 1) % CONFLICT_ROWS.len();
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
    let mut built_in: Vec<(String, Command)> = Command::all()
        .map(|command| (command.to_string(), command))
        .collect();
    built_in.sort_by(|a, b| a.0.cmp(&b.0));
    let mut externals: Vec<(String, Command)> = externals
        .iter()
        .map(|name| (format!("${name}"), Command::External(name.clone())))
        .collect();
    externals.sort_by(|a, b| a.0.cmp(&b.0));
    built_in
        .into_iter()
        .chain(externals)
        .filter(|(_, command)| command.available_in(context))
        .filter(|(name, _)| name.contains(&query))
        .map(|(_, command)| command)
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
    fn delete_confirm_names_one_item_and_counts_more() {
        let one = ChoicePopup::delete_confirm(vec![OsString::from("a.txt")]);
        assert_eq!(one.message, "Delete \"a.txt\"?");
        assert_eq!(one.title(), "Delete");
        let labels: Vec<String> = one.items().into_iter().map(|item| item.label).collect();
        assert_eq!(labels, ["Delete", "Cancel"]);
        assert_eq!(one.items()[0].detail, "Y");
        assert_eq!(one.choice_for_letter('n'), Some(1));
        assert_eq!(one.choice_for_letter('x'), None);

        let three = ChoicePopup::delete_confirm(vec![
            OsString::from("a"),
            OsString::from("b"),
            OsString::from("c"),
        ]);
        assert_eq!(three.message, "Delete 3 items?");
    }

    #[test]
    fn choice_cursor_wraps() {
        let mut popup = ChoicePopup::quit_during_operation(None);
        assert_eq!(popup.cursor, 0);
        popup.cursor_up();
        assert_eq!(popup.cursor, 1);
        popup.cursor_down();
        assert_eq!(popup.cursor, 0);
        assert_eq!(popup.choice_for_letter('Y'), None);
    }

    #[test]
    fn conflict_popup_rows_and_letters() {
        let mut popup = ConflictPopup::new(PathBuf::from("/a/x"), PathBuf::from("/b/x"), true);

        let items = popup.items();
        let rows: Vec<(&str, &str, ItemMark)> = items
            .iter()
            .map(|i| (i.label.as_str(), i.detail.as_str(), i.mark))
            .collect();
        assert_eq!(
            rows,
            [
                ("Overwrite", "O", ItemMark::None),
                ("Skip", "S", ItemMark::None),
                ("Rename", "R", ItemMark::None),
                ("Apply to all", "A", ItemMark::Check(true)),
                ("Abort", "Esc", ItemMark::None),
            ]
        );
        assert_eq!(ConflictPopup::row_for_letter('s'), Some(ConflictRow::Skip));
        assert_eq!(ConflictPopup::row_for_letter('Q'), None);
        popup.cursor_up();
        assert_eq!(popup.row_at_cursor(), ConflictRow::Abort);
        popup.cursor_down();
        assert_eq!(popup.row_at_cursor(), ConflictRow::Overwrite);
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
    fn palette_lists_the_built_in_commands_in_alphabetical_order() {
        let popup = PalettePopup::new(Context::FileList, Vec::new());
        let names: Vec<String> = popup.matches.iter().map(ToString::to_string).collect();

        assert_eq!(names[0], "bookmarks");
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]), "{names:?}");
        assert!(names.contains(&"jump_0".to_owned()));
    }

    #[test]
    fn palette_lists_external_commands_after_the_built_in_ones() {
        let mut popup = PalettePopup::new(
            Context::FileList,
            vec!["open_terminal".to_owned(), "diff".to_owned()],
        );

        let count = popup.matches.len();
        assert_eq!(
            popup.matches[count - 2..],
            [
                Command::External("diff".to_owned()),
                Command::External("open_terminal".to_owned())
            ]
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
