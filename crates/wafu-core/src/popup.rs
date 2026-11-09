use std::ffi::{OsStr, OsString};
use std::path::PathBuf;

use crate::app::Os;
use crate::command::{Command, Context};
use crate::config::Bookmark;
use crate::history::History;
use crate::key::{Key, KeyBase, NamedKey};
use crate::listing::{SortKey, SortSpec};
use crate::location::ArchiveFormat;
use crate::view::{InputField, ItemKind, PopupItem};

const ITEM_KEYS: &str = "0123456789abcdefghijklmnopqrstuvwxyz";

const SORT_ROWS: [(SortRow, &str, Option<char>); 10] = [
    (SortRow::Key(SortKey::None), "None", Some('N')),
    (SortRow::Key(SortKey::Name), "Name", Some('F')),
    (SortRow::Key(SortKey::Extension), "Extension", Some('E')),
    (SortRow::Key(SortKey::Size), "Size", Some('S')),
    (SortRow::Key(SortKey::Time), "Time", Some('T')),
    (SortRow::Key(SortKey::Natural), "Name (natural)", Some('X')),
    (SortRow::Separator, "", None),
    (SortRow::Flag(SortFlag::Descending), "Descending", Some('D')),
    (
        SortRow::Flag(SortFlag::DirsFirst),
        "Directories first",
        Some('G'),
    ),
    (
        SortRow::Flag(SortFlag::IgnoreCase),
        "Case-insensitive",
        Some('C'),
    ),
];

const DELETE_CHOICES: [Choice; 2] = [
    Choice {
        label: "Delete",
        key: 'Y',
    },
    Choice {
        label: "Cancel",
        key: 'N',
    },
];

const QUIT_CHOICES: [Choice; 2] = [
    Choice {
        label: "Quit",
        key: 'Y',
    },
    Choice {
        label: "Keep running",
        key: 'N',
    },
];

const CONFLICT_ROWS: [(ConflictRow, &str, Option<char>); 7] = [
    (ConflictRow::Separator, "", None),
    (
        ConflictRow::Choice(ConflictChoice::Overwrite),
        "Overwrite",
        Some('O'),
    ),
    (ConflictRow::Choice(ConflictChoice::Skip), "Skip", Some('S')),
    (
        ConflictRow::Choice(ConflictChoice::Rename),
        "Rename",
        Some('R'),
    ),
    (ConflictRow::Choice(ConflictChoice::Abort), "Abort", None),
    (ConflictRow::Separator, "", None),
    (ConflictRow::ApplyToAll, "Apply to all", Some('A')),
];

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

impl Popup {
    pub(crate) fn list_rows(&self) -> usize {
        match self {
            Self::Sort(popup) => popup.rows(),
            Self::Palette(popup) => popup.rows(),
            Self::List(popup) => popup.rows(),
            Self::Choice(popup) => popup.rows(),
            Self::Conflict(popup) => popup.rows(),
            Self::Input(_) | Self::Search(_) => 0,
        }
    }

    pub(crate) fn list_cursor(&self) -> usize {
        match self {
            Self::Sort(popup) => popup.cursor(),
            Self::Palette(popup) => popup.cursor(),
            Self::List(popup) => popup.cursor(),
            Self::Choice(popup) => popup.cursor(),
            Self::Conflict(popup) => popup.cursor(),
            Self::Input(_) | Self::Search(_) => 0,
        }
    }
}

/// Edits a copy of the sort settings; nothing is applied until confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortPopup {
    pub draft: SortSpec,
    /// Row in [`SortPopup::items`]: the sort keys, a separator, then the three toggles.
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
    GoToPath {
        dir: PathBuf,
    },
    Rename {
        dir: PathBuf,
        original: OsString,
    },
    Mkdir {
        dir: PathBuf,
    },
    NewFile {
        dir: PathBuf,
    },
    NewText {
        dir: PathBuf,
    },
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
    /// The name of the archive to make and its password, for the entries
    /// named in `dir`, to be put in `dest`.
    Compress {
        dir: PathBuf,
        dest: PathBuf,
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
    os: Os,
    pub query: String,
    pub matches: Vec<Command>,
    pub cursor: usize,
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
    /// The entries to delete and their directory, taken when the question was asked.
    DeleteConfirm { dir: PathBuf, names: Vec<OsString> },
    /// Asked while an operation runs. The popup that was open, if any, comes
    /// back when the operation is left running.
    QuitDuringOperation { resume: Option<Box<Popup>> },
}

/// One answer of a [`ChoicePopup`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub label: &'static str,
    /// The letter that picks the answer at once.
    pub key: char,
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

/// An answer of the conflict popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictChoice {
    Overwrite,
    Skip,
    Rename,
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
    /// A position in the pane's history and the place it held when the list opened.
    HistoryPos(usize, PathBuf),
    Path(PathBuf),
    Drive(char),
}

impl SortPopup {
    #[must_use]
    pub fn new(sort: SortSpec) -> Self {
        Self {
            draft: sort,
            cursor: SORT_ROWS
                .iter()
                .position(|(row, _, _)| *row == SortRow::Key(sort.key))
                .unwrap_or(0),
        }
    }

    #[must_use]
    pub fn items(&self) -> Vec<PopupItem> {
        SORT_ROWS
            .iter()
            .enumerate()
            .map(|(index, &(row, label, key))| match row {
                SortRow::Separator => PopupItem::separator(),
                _ => PopupItem {
                    label: if row == SortRow::Key(self.draft.key) {
                        format!("{label} *")
                    } else {
                        label.to_owned()
                    },
                    detail: key.map(String::from).unwrap_or_default(),
                    kind: self.kind(index),
                },
            })
            .collect()
    }

    fn flag(&self, flag: SortFlag) -> bool {
        match flag {
            SortFlag::Descending => self.draft.descending,
            SortFlag::DirsFirst => self.draft.dirs_first,
            SortFlag::IgnoreCase => self.draft.ignore_case,
        }
    }

    fn flag_mut(&mut self, flag: SortFlag) -> &mut bool {
        match flag {
            SortFlag::Descending => &mut self.draft.descending,
            SortFlag::DirsFirst => &mut self.draft.dirs_first,
            SortFlag::IgnoreCase => &mut self.draft.ignore_case,
        }
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
    pub fn compress(
        dir: PathBuf,
        dest: PathBuf,
        names: Vec<OsString>,
        initial_name: String,
    ) -> Self {
        let select = stem_selection(&initial_name);
        Self {
            kind: InputKind::Compress { dir, dest, names },
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
            _ => None,
        }
    }

    #[must_use]
    pub fn note(&self) -> Option<String> {
        match self.kind {
            InputKind::Compress { .. } => {
                Some("zip, tar, tar.gz, tar.bz2, or tar.xz; only zip takes a password".to_owned())
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
            Self::GoToPath { .. } => "Go to path",
            Self::Rename { .. } | Self::ConflictRename { .. } => "Rename",
            Self::Mkdir { .. } => "Make directory",
            Self::NewFile { .. } => "New file",
            Self::NewText { .. } => "New text",
            Self::ArchivePassword { .. } => "Password",
            Self::Compress { .. } => "Compress",
        }
    }
}

impl ChoicePopup {
    /// Asks before deleting; the first answer deletes.
    #[must_use]
    pub fn delete_confirm(dir: PathBuf, names: Vec<OsString>) -> Self {
        let message = match names.as_slice() {
            [name] => format!("Delete \"{}\"?", name.to_string_lossy()),
            _ => format!("Delete {} items?", names.len()),
        };
        Self {
            kind: ChoiceKind::DeleteConfirm { dir, names },
            message,
            cursor: 1,
        }
    }

    /// Asks whether to quit while an operation runs; the first answer cancels it and quits.
    #[must_use]
    pub fn quit_during_operation(resume: Option<Popup>) -> Self {
        Self {
            kind: ChoiceKind::QuitDuringOperation {
                resume: resume.map(Box::new),
            },
            message: "An operation is in progress. Quit anyway?".to_owned(),
            cursor: 1,
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
        std::iter::once(PopupItem::separator())
            .chain(self.choices().iter().map(|choice| PopupItem {
                label: choice.label.to_owned(),
                detail: choice.key.to_string(),
                kind: ItemKind::Choice,
            }))
            .collect()
    }
}

impl ConflictPopup {
    #[must_use]
    pub fn new(source: PathBuf, dest: PathBuf, all: bool) -> Self {
        Self {
            source,
            dest,
            all,
            cursor: 1,
        }
    }

    #[must_use]
    pub fn items(&self) -> Vec<PopupItem> {
        CONFLICT_ROWS
            .iter()
            .enumerate()
            .map(|(index, (row, label, key))| match row {
                ConflictRow::Separator => PopupItem::separator(),
                _ => PopupItem {
                    label: (*label).to_owned(),
                    detail: key.map_or_else(|| "Esc".to_owned(), String::from),
                    kind: self.kind(index),
                },
            })
            .collect()
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
    pub fn history(history: &History, in_archive: bool) -> Self {
        let items = history
            .items()
            .iter()
            .enumerate()
            .rev()
            .zip(item_keys())
            .map(|((pos, path), key)| ListItem {
                key,
                label: path.display().to_string(),
                current: !in_archive && pos == history.pos(),
                target: ListTarget::HistoryPos(pos, path.clone()),
            })
            .collect();
        Self {
            kind: ListKind::History,
            items,
            cursor: history.items().len().saturating_sub(history.pos() + 1),
        }
    }

    /// Bookmarks in the configured order, their paths already expanded.
    #[must_use]
    pub fn bookmarks(bookmarks: Vec<Bookmark>) -> Self {
        let items = bookmarks
            .into_iter()
            .zip(item_keys())
            .map(|(bookmark, key)| ListItem {
                key,
                label: bookmark.name,
                current: false,
                target: ListTarget::Path(bookmark.path),
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
                kind: ItemKind::Choice,
            })
            .collect()
    }

    #[must_use]
    pub fn selected(&self) -> Option<&ListItem> {
        self.items.get(self.cursor)
    }
}

impl PalettePopup {
    #[must_use]
    pub fn new(context: Context, os: Os, externals: Vec<String>) -> Self {
        Self {
            context,
            os,
            query: String::new(),
            matches: matches(context, os, &externals, ""),
            cursor: 0,
            externals,
        }
    }

    pub fn set_query(&mut self, query: String) {
        self.matches = matches(self.context, self.os, &self.externals, &query);
        self.query = query;
        self.cursor = 0;
    }

    #[must_use]
    pub fn selected(&self) -> Option<Command> {
        self.matches.get(self.cursor).cloned()
    }
}

/// The part of a file name to select when editing it: everything but the
/// extension, or nothing special when there is none.
#[must_use]
pub fn stem_selection(name: &str) -> Option<(usize, usize)> {
    let stem = ArchiveFormat::stem_of(OsStr::new(name)).len();
    (stem > 0 && stem < name.len()).then_some((0, stem))
}

// The popups that pick a row. The rules for keys and the mouse live in
// `menu_input` alone; an implementation only describes its rows and what
// running one does. Separators are rows here too, so that the UI's row
// numbers are the core's and a click needs no translation.
pub(crate) trait Menu {
    type Pick;

    fn rows(&self) -> usize;
    fn kind(&self, row: usize) -> ItemKind;
    fn cursor(&self) -> usize;
    fn set_cursor(&mut self, row: usize);
    fn row_for_key(&self, key: char) -> Option<usize>;
    fn run(&mut self, row: usize) -> Response<Self::Pick>;

    fn cursor_up(&mut self) {
        let rows = self.rows();
        let mut row = self.cursor();
        for _ in 0..rows {
            row = row.checked_sub(1).unwrap_or(rows - 1);
            if self.kind(row) != ItemKind::Separator {
                self.set_cursor(row);
                return;
            }
        }
    }

    fn cursor_down(&mut self) {
        let rows = self.rows();
        let mut row = self.cursor();
        for _ in 0..rows {
            row = (row + 1) % rows;
            if self.kind(row) != ItemKind::Separator {
                self.set_cursor(row);
                return;
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Response<P> {
    Stay,
    Close,
    Pick(P),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MenuInput {
    Key(Key),
    Click { row: usize },
    DoubleClick { row: usize },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortRow {
    Key(SortKey),
    Separator,
    Flag(SortFlag),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortFlag {
    Descending,
    DirsFirst,
    IgnoreCase,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictRow {
    Separator,
    Choice(ConflictChoice),
    ApplyToAll,
}

impl Menu for SortPopup {
    type Pick = SortSpec;

    fn rows(&self) -> usize {
        SORT_ROWS.len()
    }

    fn kind(&self, row: usize) -> ItemKind {
        match SORT_ROWS[row].0 {
            SortRow::Key(_) => ItemKind::Choice,
            SortRow::Separator => ItemKind::Separator,
            SortRow::Flag(flag) => ItemKind::Check(self.flag(flag)),
        }
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_cursor(&mut self, row: usize) {
        self.cursor = row;
    }

    fn row_for_key(&self, key: char) -> Option<usize> {
        SORT_ROWS.iter().position(|(_, _, row_key)| {
            row_key.is_some_and(|row_key| row_key.eq_ignore_ascii_case(&key))
        })
    }

    fn run(&mut self, row: usize) -> Response<SortSpec> {
        match SORT_ROWS[row].0 {
            SortRow::Key(key) => {
                self.draft.key = key;
                Response::Pick(self.draft)
            }
            SortRow::Separator => Response::Stay,
            SortRow::Flag(flag) => {
                let flag = self.flag_mut(flag);
                *flag = !*flag;
                Response::Stay
            }
        }
    }
}

impl Menu for ChoicePopup {
    type Pick = usize;

    fn rows(&self) -> usize {
        1 + self.choices().len()
    }

    fn kind(&self, row: usize) -> ItemKind {
        if row == 0 {
            ItemKind::Separator
        } else {
            ItemKind::Choice
        }
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_cursor(&mut self, row: usize) {
        self.cursor = row;
    }

    fn row_for_key(&self, key: char) -> Option<usize> {
        let key = key.to_ascii_uppercase();
        self.choices()
            .iter()
            .position(|choice| choice.key == key)
            .map(|index| index + 1)
    }

    fn run(&mut self, row: usize) -> Response<usize> {
        match row.checked_sub(1) {
            Some(choice) if choice < self.choices().len() => Response::Pick(choice),
            _ => Response::Stay,
        }
    }
}

impl Menu for ConflictPopup {
    type Pick = ConflictChoice;

    fn rows(&self) -> usize {
        CONFLICT_ROWS.len()
    }

    fn kind(&self, row: usize) -> ItemKind {
        match CONFLICT_ROWS[row].0 {
            ConflictRow::Separator => ItemKind::Separator,
            ConflictRow::Choice(_) => ItemKind::Choice,
            ConflictRow::ApplyToAll => ItemKind::Check(self.all),
        }
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_cursor(&mut self, row: usize) {
        self.cursor = row;
    }

    fn row_for_key(&self, key: char) -> Option<usize> {
        CONFLICT_ROWS.iter().position(|(_, _, row_key)| {
            row_key.is_some_and(|row_key| row_key.eq_ignore_ascii_case(&key))
        })
    }

    fn run(&mut self, row: usize) -> Response<ConflictChoice> {
        match CONFLICT_ROWS[row].0 {
            ConflictRow::Separator => Response::Stay,
            ConflictRow::Choice(choice) => Response::Pick(choice),
            ConflictRow::ApplyToAll => {
                self.all = !self.all;
                Response::Stay
            }
        }
    }
}

impl Menu for ListPopup {
    type Pick = ListItem;

    fn rows(&self) -> usize {
        self.items.len()
    }

    fn kind(&self, _row: usize) -> ItemKind {
        ItemKind::Choice
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_cursor(&mut self, row: usize) {
        self.cursor = row;
    }

    fn row_for_key(&self, key: char) -> Option<usize> {
        let key = key.to_ascii_lowercase();
        self.items.iter().position(|item| item.key == Some(key))
    }

    fn run(&mut self, row: usize) -> Response<ListItem> {
        match self.items.get(row) {
            Some(item) => Response::Pick(item.clone()),
            None => Response::Close,
        }
    }
}

impl Menu for PalettePopup {
    type Pick = Command;

    fn rows(&self) -> usize {
        self.matches.len()
    }

    fn kind(&self, _row: usize) -> ItemKind {
        ItemKind::Choice
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_cursor(&mut self, row: usize) {
        self.cursor = row;
    }

    fn row_for_key(&self, _key: char) -> Option<usize> {
        None
    }

    fn run(&mut self, row: usize) -> Response<Command> {
        self.matches
            .get(row)
            .cloned()
            .map_or(Response::Close, Response::Pick)
    }
}

pub(crate) fn menu_input<M: Menu>(menu: &mut M, input: MenuInput) -> Response<M::Pick> {
    match input {
        MenuInput::Key(key) => match key.base {
            KeyBase::Named(NamedKey::Escape) => Response::Close,
            KeyBase::Named(NamedKey::Enter) => menu.run(menu.cursor()),
            KeyBase::Named(NamedKey::Space)
                if matches!(menu.kind(menu.cursor()), ItemKind::Check(_)) =>
            {
                menu.run(menu.cursor())
            }
            KeyBase::Named(NamedKey::Up) => {
                menu.cursor_up();
                Response::Stay
            }
            KeyBase::Named(NamedKey::Down) => {
                menu.cursor_down();
                Response::Stay
            }
            KeyBase::Char(c) if !key.ctrl && !key.alt => match menu.row_for_key(c) {
                Some(row) => menu.run(row),
                None => Response::Stay,
            },
            _ => Response::Stay,
        },
        MenuInput::Click { row } => {
            if selectable(menu, row) {
                menu.set_cursor(row);
            }
            Response::Stay
        }
        MenuInput::DoubleClick { row } => {
            if selectable(menu, row) {
                menu.set_cursor(row);
                menu.run(row)
            } else {
                Response::Stay
            }
        }
    }
}

fn selectable<M: Menu>(menu: &M, row: usize) -> bool {
    row < menu.rows() && menu.kind(row) != ItemKind::Separator
}

fn item_keys() -> impl Iterator<Item = Option<char>> {
    ITEM_KEYS.chars().map(Some).chain(std::iter::repeat(None))
}

fn matches(context: Context, os: Os, externals: &[String], query: &str) -> Vec<Command> {
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
        .filter(|(_, command)| command.available_in(context) && command.available_on(os))
        .filter(|(name, _)| name.to_lowercase().contains(&query))
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
        let rows: Vec<(&str, &str, ItemKind)> = items
            .iter()
            .map(|i| (i.label.as_str(), i.detail.as_str(), i.kind))
            .collect();
        assert_eq!(
            rows,
            [
                ("None", "N", ItemKind::Choice),
                ("Name", "F", ItemKind::Choice),
                ("Extension", "E", ItemKind::Choice),
                ("Size *", "S", ItemKind::Choice),
                ("Time", "T", ItemKind::Choice),
                ("Name (natural)", "X", ItemKind::Choice),
                ("", "", ItemKind::Separator),
                ("Descending", "D", ItemKind::Check(false)),
                ("Directories first", "G", ItemKind::Check(true)),
                ("Case-insensitive", "C", ItemKind::Check(true)),
            ]
        );
        assert_eq!(popup.rows(), items.len());
    }

    #[test]
    fn sort_popup_letters_flip_toggles_and_pick_a_key_with_them() {
        let mut popup = SortPopup::new(SortSpec::default());

        assert_eq!(menu_input(&mut popup, key("D")), Response::Stay);
        assert_eq!(menu_input(&mut popup, key("G")), Response::Stay);
        assert_eq!(menu_input(&mut popup, key("C")), Response::Stay);
        let toggled = SortSpec {
            descending: true,
            dirs_first: false,
            ignore_case: false,
            ..SortSpec::default()
        };
        assert_eq!(popup.draft, toggled);
        assert_eq!(
            menu_input(&mut popup, key("E")),
            Response::Pick(SortSpec {
                key: SortKey::Extension,
                ..toggled
            })
        );
    }

    #[test]
    fn password_popup_names_the_archive_and_says_when_the_password_was_wrong() {
        let archive = PathBuf::from("/a/b.zip");

        let first = InputPopup::password(archive.clone(), false, PasswordPurpose::Operation);
        let retry = InputPopup::password(archive, true, PasswordPurpose::Operation);

        assert_eq!(first.message().as_deref(), Some("b.zip"));
        assert_eq!(retry.message().as_deref(), Some("Wrong password for b.zip"));
    }

    #[test]
    fn choice_cursor_skips_the_separator_and_wraps() {
        let mut popup = ChoicePopup::quit_during_operation(None);
        assert_eq!(popup.cursor, 1);
        popup.cursor_up();
        assert_eq!(popup.cursor, 2);
        popup.cursor_down();
        assert_eq!(popup.cursor, 1);
        assert_eq!(popup.run(1), Response::Pick(0));
        assert_eq!(popup.run(2), Response::Pick(1));
        assert_eq!(popup.run(0), Response::Stay);
        assert_eq!(popup.row_for_key('Y'), Some(1));
    }

    #[test]
    fn delete_confirm_names_a_single_entry_and_counts_several() {
        let one = ChoicePopup::delete_confirm(PathBuf::from("/d"), vec!["a".into()]);
        let two = ChoicePopup::delete_confirm(PathBuf::from("/d"), vec!["a".into(), "b".into()]);

        assert_eq!(one.message, "Delete \"a\"?");
        assert_eq!(two.message, "Delete 2 items?");
        assert_eq!(one.cursor, 1);
    }

    #[test]
    fn conflict_popup_flips_apply_to_all_and_picks_the_rest() {
        let mut popup = ConflictPopup::new(PathBuf::from("/a/x"), PathBuf::from("/b/x"), false);

        assert_eq!(popup.run(6), Response::Stay);
        assert!(popup.all);
        assert_eq!(popup.run(3), Response::Pick(ConflictChoice::Rename));
        assert_eq!(popup.run(4), Response::Pick(ConflictChoice::Abort));
    }

    #[test]
    fn conflict_popup_shows_the_keys_and_the_apply_to_all_state() {
        let popup = ConflictPopup::new(PathBuf::from("/a/x"), PathBuf::from("/b/x"), true);

        let items = popup.items();
        assert_eq!(items[1].detail, "O");
        assert_eq!(
            (items[4].label.as_str(), items[4].detail.as_str()),
            ("Abort", "Esc")
        );
        assert_eq!(items[6].kind, ItemKind::Check(true));
        assert_eq!(popup.cursor, 1);
    }

    #[test]
    fn menu_input_moves_the_cursor_with_up_and_down() {
        let mut popup = conflict_popup();

        assert_eq!(menu_input(&mut popup, key("Down")), Response::Stay);
        assert_eq!(popup.cursor, 2);
        assert_eq!(menu_input(&mut popup, key("Up")), Response::Stay);
        assert_eq!(popup.cursor, 1);
    }

    #[test]
    fn menu_input_enter_runs_the_row_under_the_cursor() {
        let mut popup = conflict_popup();

        popup.cursor = 2;
        assert_eq!(
            menu_input(&mut popup, key("Enter")),
            Response::Pick(ConflictChoice::Skip)
        );
        popup.cursor = 6;
        assert_eq!(menu_input(&mut popup, key("Enter")), Response::Stay);
        assert!(popup.all);
    }

    #[test]
    fn menu_input_space_toggles_a_check_row_only() {
        let mut popup = conflict_popup();

        assert_eq!(menu_input(&mut popup, key("Space")), Response::Stay);
        assert_eq!(popup, conflict_popup());
        popup.cursor = 6;
        assert_eq!(menu_input(&mut popup, key("Space")), Response::Stay);
        assert!(popup.all);
    }

    #[test]
    fn menu_input_letters_run_their_row_without_moving_the_cursor() {
        let mut popup = conflict_popup();

        assert_eq!(menu_input(&mut popup, key("A")), Response::Stay);
        assert_eq!((popup.cursor, popup.all), (1, true));
        assert_eq!(menu_input(&mut popup, key("X")), Response::Stay);
        assert_eq!(
            menu_input(&mut popup, key("Shift+O")),
            Response::Pick(ConflictChoice::Overwrite)
        );
    }

    #[test]
    fn menu_input_ignores_letters_with_ctrl_or_alt() {
        let mut popup = conflict_popup();

        assert_eq!(menu_input(&mut popup, key("Ctrl+O")), Response::Stay);
        assert_eq!(menu_input(&mut popup, key("Alt+O")), Response::Stay);
    }

    #[test]
    fn menu_input_closes_on_escape() {
        let mut popup = conflict_popup();

        assert_eq!(menu_input(&mut popup, key("Esc")), Response::Close);
    }

    #[test]
    fn menu_input_clicks_move_the_cursor_and_double_clicks_run_the_row() {
        let mut popup = ConflictPopup::new(PathBuf::from("/a/x"), PathBuf::from("/b/x"), false);

        assert_eq!(
            menu_input(&mut popup, MenuInput::Click { row: 6 }),
            Response::Stay
        );
        assert_eq!((popup.cursor, popup.all), (6, false));
        assert_eq!(
            menu_input(&mut popup, MenuInput::Click { row: 5 }),
            Response::Stay
        );
        assert_eq!(popup.cursor, 6);
        assert_eq!(
            menu_input(&mut popup, MenuInput::DoubleClick { row: 5 }),
            Response::Stay
        );
        assert_eq!(
            menu_input(&mut popup, MenuInput::Click { row: 7 }),
            Response::Stay
        );
        assert_eq!(popup.cursor, 6);
        assert_eq!(
            menu_input(&mut popup, MenuInput::DoubleClick { row: 6 }),
            Response::Stay
        );
        assert!(popup.all);
        assert_eq!(
            menu_input(&mut popup, MenuInput::DoubleClick { row: 3 }),
            Response::Pick(ConflictChoice::Rename)
        );
        assert_eq!(popup.cursor, 3);
    }

    #[test]
    fn palette_leaves_out_external_commands_outside_the_file_list() {
        let popup = PalettePopup::new(Context::Image, Os::Windows, vec!["x".to_owned()]);

        assert!(popup.matches.contains(&Command::CursorUp));
        assert!(!popup.matches.contains(&Command::External("x".to_owned())));
    }

    #[test]
    fn palette_lists_the_built_in_commands_in_alphabetical_order() {
        let popup = PalettePopup::new(Context::FileList, Os::Windows, Vec::new());
        let names: Vec<String> = popup.matches.iter().map(ToString::to_string).collect();

        assert_eq!(names[0], "bookmarks");
        assert!(names.windows(2).all(|pair| pair[0] < pair[1]), "{names:?}");
        assert!(names.contains(&"jump_0".to_owned()));
    }

    #[test]
    fn palette_lists_external_commands_after_the_built_in_ones() {
        let mut popup = PalettePopup::new(
            Context::FileList,
            Os::Windows,
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
    fn palette_filters_external_commands_ignoring_case() {
        let mut popup =
            PalettePopup::new(Context::FileList, Os::Windows, vec!["MyTerm".to_owned()]);

        popup.set_query("myterm".to_owned());
        assert_eq!(popup.matches, [Command::External("MyTerm".to_owned())]);
        popup.set_query("MYTERM".to_owned());
        assert_eq!(popup.matches, [Command::External("MyTerm".to_owned())]);
    }

    #[test]
    fn palette_matches_the_query_anywhere_in_the_name() {
        let mut popup = PalettePopup::new(Context::FileList, Os::Windows, Vec::new());

        popup.set_query("marks".to_owned());

        assert!(popup.matches.contains(&Command::ClearMarks));
    }

    #[test]
    fn palette_leaves_out_windows_only_commands_on_other_systems() {
        let popup = PalettePopup::new(Context::FileList, Os::Linux, Vec::new());

        assert!(!popup.matches.contains(&Command::Drives));
    }

    #[test]
    fn list_popup_gives_keys_to_the_first_36_items_only() {
        let bookmarks = (0..40).map(|i| bookmark(&format!("/{i}"))).collect();

        let popup = ListPopup::bookmarks(bookmarks);

        assert_eq!(popup.items[0].key, Some('0'));
        assert_eq!(popup.items[9].key, Some('9'));
        assert_eq!(popup.items[10].key, Some('a'));
        assert_eq!(popup.items[35].key, Some('z'));
        assert_eq!(popup.items[36].key, None);
        assert_eq!(popup.items()[35].detail, "Z");
        assert_eq!(popup.items()[36].detail, "");
        assert_eq!(popup.row_for_key('Z'), Some(35));
        assert_eq!(popup.row_for_key('!'), None);
    }

    #[test]
    fn list_popup_of_history_lists_the_newest_first() {
        let mut history = History::new();
        for path in ["/a", "/b", "/c"] {
            history.visit(PathBuf::from(path));
        }

        let popup = ListPopup::history(&history, false);

        let item = |key, pos, path: &str| ListItem {
            key: Some(key),
            label: PathBuf::from(path).display().to_string(),
            current: pos == 2,
            target: ListTarget::HistoryPos(pos, PathBuf::from(path)),
        };
        assert_eq!(
            popup.items,
            [item('0', 2, "/c"), item('1', 1, "/b"), item('2', 0, "/a")]
        );
        assert_eq!(popup.cursor, 0);
    }

    #[test]
    fn list_popup_of_history_inside_an_archive_has_no_current_place() {
        let mut history = History::new();
        history.visit(PathBuf::from("/a"));
        history.visit(PathBuf::from("/b"));
        history.visit(PathBuf::from("/c"));
        history.set_pos(1);

        let popup = ListPopup::history(&history, true);

        assert!(popup.items.iter().all(|item| !item.current));
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
    fn list_popup_closes_on_enter_when_empty() {
        let mut popup = ListPopup::bookmarks(Vec::new());

        assert_eq!(menu_input(&mut popup, key("Enter")), Response::Close);
    }

    #[test]
    fn palette_cursor_wraps_and_handles_no_matches() {
        let mut popup = PalettePopup::new(Context::FileList, Os::Windows, Vec::new());
        popup.set_query("jump_".to_owned());
        assert_eq!(popup.matches.len(), 36);

        popup.cursor_up();
        assert_eq!(popup.cursor, 35);
        popup.cursor_down();
        assert_eq!(popup.cursor, 0);

        popup.set_query("nothing matches this".to_owned());
        popup.cursor_down();
        assert_eq!(popup.selected(), None);
        assert_eq!(popup.run(0), Response::Close);
    }

    #[test]
    fn stem_selection_selects_the_name_before_a_compound_extension() {
        assert_eq!(stem_selection("a.tar.gz"), Some((0, 1)));
    }

    #[test]
    fn stem_selection_selects_nothing_special_without_an_extension() {
        assert_eq!(stem_selection("archive"), None);
    }

    fn key(text: &str) -> MenuInput {
        MenuInput::Key(text.parse().unwrap())
    }

    fn conflict_popup() -> ConflictPopup {
        ConflictPopup::new(PathBuf::from("/a/x"), PathBuf::from("/b/x"), false)
    }

    fn bookmark(path: &str) -> Bookmark {
        Bookmark {
            name: path.to_owned(),
            path: PathBuf::from(path),
        }
    }
}
