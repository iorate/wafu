use std::ffi::{OsStr, OsString};
use std::ops::Range;
use std::path::{Path, PathBuf};

use encoding_rs::Encoding;

use crate::app::{Os, Side};
use crate::batch_rename::{RenameInput, RenamePlan};
use crate::command::{Command, Context};
use crate::compare::{CompareCondition, CompareSpec};
use crate::config::Bookmark;
use crate::entry::Entry;
use crate::history::History;
use crate::key::{Key, KeyBase, NamedKey};
use crate::listing::{SortKey, SortSpec};
use crate::location::{ArchiveFormat, Location, Passwords};
use crate::pane::HistoryMove;
use crate::path::resolve_input;
use crate::preview::ENCODINGS;
use crate::scroll::{self, LineRows};
use crate::text::nfc;
use crate::view::{InputField, ItemKind, PopupItem, PopupLine, PopupLines};

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

const COMPARE_ROWS: [(CompareRow, &str, Option<char>); 6] = [
    (
        CompareRow::Condition(CompareCondition::Missing),
        "Missing",
        Some('M'),
    ),
    (
        CompareRow::Condition(CompareCondition::Newer),
        "Missing or newer",
        Some('N'),
    ),
    (
        CompareRow::Condition(CompareCondition::Different),
        "Missing or different",
        Some('D'),
    ),
    (CompareRow::Separator, "", None),
    (
        CompareRow::Flag(CompareFlag::IncludeDirs),
        "Include directories",
        Some('I'),
    ),
    (
        CompareRow::Flag(CompareFlag::IgnoreCase),
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

const REPLACE_HINT: &str = "Replaces every occurrence, ignoring case";
const TEMPLATE_HINT: &str = "Placeholders: {name}, {ext}, {001} (counter from 001)";
const INITIAL_TEMPLATE: &str = "{name}{ext}";

/// A modal that takes all keys while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    Sort(SortPopup),
    Compare(ComparePopup),
    Input(InputPopup),
    Search(SearchPopup),
    Palette(PalettePopup),
    List(ListPopup),
    Choice(ChoicePopup),
    Conflict(ConflictPopup),
    Rename(RenamePopup),
}

impl Popup {
    pub(crate) fn list_rows(&self) -> usize {
        match self {
            Self::Sort(popup) => popup.rows(),
            Self::Compare(popup) => popup.rows(),
            Self::Palette(popup) => popup.rows(),
            Self::List(popup) => popup.rows(),
            Self::Choice(popup) => popup.rows(),
            Self::Conflict(popup) => popup.rows(),
            Self::Input(_) | Self::Search(_) | Self::Rename(_) => 0,
        }
    }

    pub(crate) fn list_cursor(&self) -> usize {
        match self {
            Self::Sort(popup) => popup.cursor(),
            Self::Compare(popup) => popup.cursor(),
            Self::Palette(popup) => popup.cursor(),
            Self::List(popup) => popup.cursor(),
            Self::Choice(popup) => popup.cursor(),
            Self::Conflict(popup) => popup.cursor(),
            Self::Input(_) | Self::Search(_) | Self::Rename(_) => 0,
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

/// Edits a copy of the compare settings; nothing is applied until confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparePopup {
    pub draft: CompareSpec,
    /// Row in [`ComparePopup::items`]: the conditions, a separator, then the two toggles.
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
    /// The pattern to find below `dir`.
    Find {
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
    /// A command line to start in the active pane's directory, from `run_file`.
    RunFile,
    /// A command line to start in the active pane's directory, from `run_command`.
    RunCommand,
    /// The "Rename" choice of a conflict; Esc goes back to the conflict popup.
    ConflictRename {
        source: PathBuf,
        dest: PathBuf,
        all: bool,
    },
    /// An encrypted entry was met in the archive `archive`, at `level` of
    /// what was read (see [`Passwords`]). `retry` means the last password was
    /// wrong.
    ArchivePassword {
        archive: PathBuf,
        level: usize,
        retry: bool,
        purpose: PasswordPurpose,
    },
    /// The name of the archive to make and its password, for the entries
    /// named in `dir`, to be put in `dest`: the directory of the same pane
    /// when `here`, or else that of the other.
    Compress {
        dir: PathBuf,
        dest: PathBuf,
        names: Vec<OsString>,
        here: bool,
    },
}

/// Who asked for the password, and so who gets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasswordPurpose {
    Operation,
    Preview,
    /// The listing of a pane, read with `passwords`: of the move in
    /// `navigation`, or of where the pane is when it is `None`.
    Listing {
        side: Side,
        passwords: Passwords,
        navigation: Option<Box<PendingNavigation>>,
    },
}

/// A move to make again once a password is given.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingNavigation {
    pub target: Location,
    pub cursor: Option<OsString>,
    pub history: HistoryMove,
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

/// A list to pick from: places (history, bookmarks, or drives), what a find
/// found, or the encodings of a document preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPopup {
    pub kind: ListKind,
    pub items: Vec<ListItem>,
    pub query: String,
    /// The items shown, as indices into `items`: those whose label has the query.
    pub matches: Vec<usize>,
    /// A row of `matches`.
    pub cursor: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListKind {
    /// The history of the pane on that side.
    History(Side),
    Bookmarks,
    Drives,
    Encodings,
    Found,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListItem {
    /// The key that picks the item at once, if it has one.
    pub key: Option<char>,
    pub label: String,
    /// Where the pane already is, or the encoding the preview is decoded
    /// with; the list opens on it.
    pub current: bool,
    pub target: ListTarget,
    /// What the configuration gives for the item, as written there.
    pub source: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListTarget {
    /// A position in the pane's history and the place it held when the list opened.
    HistoryPos(usize, PathBuf),
    Path(PathBuf),
    /// An entry to put the cursor on, in its directory.
    Entry(PathBuf),
    Drive(char),
    /// The encoding to decode the previewed document with; `None` detects it.
    Encoding(Option<&'static Encoding>),
}

/// Renames entries of one directory at once by one rule, with the list of
/// the targets whose name changes and what each becomes. The targets are
/// fixed when it opens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePopup {
    mode: RenameMode,
    pub dir: PathBuf,
    /// In display order.
    pub targets: Vec<Entry>,
    // The pane's listing does not hold the targets' siblings when the target
    // is the pane's own directory.
    siblings: bool,
    pub fields: Vec<InputField>,
    pub plan: RenamePlan,
    // Counted among the items the list shows, not among the targets.
    top: usize,
    rows: LineRows,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameMode {
    /// `Find` and `Replace`.
    Replace,
    /// One template of the whole name.
    Template,
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

impl ComparePopup {
    #[must_use]
    pub fn new(compare: CompareSpec) -> Self {
        Self {
            draft: compare,
            cursor: COMPARE_ROWS
                .iter()
                .position(|(row, _, _)| *row == CompareRow::Condition(compare.condition))
                .unwrap_or(0),
        }
    }

    #[must_use]
    pub fn items(&self) -> Vec<PopupItem> {
        COMPARE_ROWS
            .iter()
            .enumerate()
            .map(|(index, &(row, label, key))| match row {
                CompareRow::Separator => PopupItem::separator(),
                _ => PopupItem {
                    label: if row == CompareRow::Condition(self.draft.condition) {
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

    fn flag(&self, flag: CompareFlag) -> bool {
        match flag {
            CompareFlag::IncludeDirs => self.draft.include_dirs,
            CompareFlag::IgnoreCase => self.draft.ignore_case,
        }
    }

    fn flag_mut(&mut self, flag: CompareFlag) -> &mut bool {
        match flag {
            CompareFlag::IncludeDirs => &mut self.draft.include_dirs,
            CompareFlag::IgnoreCase => &mut self.draft.ignore_case,
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
    pub fn password(archive: PathBuf, level: usize, retry: bool, purpose: PasswordPurpose) -> Self {
        Self {
            kind: InputKind::ArchivePassword {
                archive,
                level,
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
        here: bool,
        initial_name: String,
    ) -> Self {
        let select = stem_selection(&initial_name);
        Self {
            kind: InputKind::Compress {
                dir,
                dest,
                names,
                here,
            },
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
            InputKind::ArchivePassword { archive, .. } => Some(
                archive
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ),
            _ => None,
        }
    }

    #[must_use]
    pub fn note(&self) -> Option<String> {
        match self.kind {
            InputKind::ArchivePassword { retry: true, .. } => Some("Wrong password".to_owned()),
            InputKind::Compress { .. } => Some(
                "Formats: zip, tar, tar.gz, tar.bz2, tar.xz; only zip takes a password".to_owned(),
            ),
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
            Self::Find { .. } => "Find",
            Self::Rename { .. } | Self::ConflictRename { .. } => "Rename",
            Self::Mkdir { .. } => "Make directory",
            Self::NewFile { .. } => "New file",
            Self::NewText { .. } => "New text",
            Self::RunFile | Self::RunCommand => "Run",
            Self::ArchivePassword { .. } => "Password",
            Self::Compress { here: false, .. } => "Compress",
            Self::Compress { here: true, .. } => "Compress (here)",
        }
    }
}

impl ChoicePopup {
    /// Asks before deleting; the first answer deletes.
    #[must_use]
    pub fn delete_confirm(dir: PathBuf, names: Vec<OsString>) -> Self {
        let message = match names.as_slice() {
            [name] => name.to_string_lossy().into_owned(),
            _ => format!("{} items", names.len()),
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
            message: "An operation is in progress".to_owned(),
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
            matches: (0..items.len()).collect(),
            items,
            query: String::new(),
            cursor,
        }
    }

    /// The history of the pane that picks from it, newest first.
    #[must_use]
    pub fn history(side: Side, history: &History, in_archive: bool) -> Self {
        Self::history_with(side, history, |pos, path| ListItem {
            key: None,
            label: path.display().to_string(),
            current: !in_archive && pos == history.pos(),
            target: ListTarget::HistoryPos(pos, path.clone()),
            source: None,
        })
    }

    /// The history of the pane on `side` for the other pane to pick from, newest
    /// first. A pick is a place to visit, not a position in the picking pane's
    /// own history.
    #[must_use]
    pub fn other_history(side: Side, history: &History) -> Self {
        Self::history_with(side, history, |_, path| ListItem {
            key: None,
            label: path.display().to_string(),
            current: false,
            target: ListTarget::Path(path.clone()),
            source: None,
        })
    }

    /// Bookmarks in the configured order; a pick goes to the path with `~`
    /// taken from `home` and any other relative path from the bookmark's `dir`.
    #[must_use]
    pub fn bookmarks(bookmarks: &[Bookmark], home: &Path) -> Self {
        let items = bookmarks
            .iter()
            .map(|bookmark| {
                let path = bookmark.path.to_string_lossy();
                ListItem {
                    key: bookmark.key,
                    label: bookmark.name.clone(),
                    current: false,
                    target: ListTarget::Path(resolve_input(&path, home, &bookmark.dir)),
                    source: Some(path.into_owned()),
                }
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
                source: None,
            })
            .collect();
        Self::new(ListKind::Drives, items)
    }

    /// What a find found below the directory, in the order given.
    #[must_use]
    pub fn found(dir: &Path, paths: Vec<PathBuf>) -> Self {
        let items = paths
            .into_iter()
            .map(|path| ListItem {
                key: None,
                label: path
                    .strip_prefix(dir)
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
                current: false,
                target: ListTarget::Entry(path),
                source: None,
            })
            .collect();
        Self::new(ListKind::Found, items)
    }

    /// Auto-detection first, then every encoding, with the one named `current` marked.
    #[must_use]
    pub fn encodings(current: &str) -> Self {
        let items = std::iter::once(ListItem {
            key: None,
            label: "Auto-detect".to_owned(),
            current: false,
            target: ListTarget::Encoding(None),
            source: None,
        })
        .chain(ENCODINGS.iter().map(|&encoding| ListItem {
            key: None,
            label: encoding.name().to_owned(),
            current: encoding.name() == current,
            target: ListTarget::Encoding(Some(encoding)),
            source: None,
        }))
        .collect();
        Self::new(ListKind::Encodings, items)
    }

    pub fn set_query(&mut self, query: String) {
        let needle = nfc(&query).to_lowercase();
        self.matches = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| nfc(&item.label).to_lowercase().contains(&needle))
            .map(|(index, _)| index)
            .collect();
        self.query = query;
        self.cursor = 0;
    }

    #[must_use]
    pub fn title(&self) -> &'static str {
        match self.kind {
            ListKind::History(Side::Left) => "History (left)",
            ListKind::History(Side::Right) => "History (right)",
            ListKind::Bookmarks => "Bookmarks",
            ListKind::Drives => "Drives",
            ListKind::Encodings => "Encoding",
            ListKind::Found => "Find",
        }
    }

    /// What to show instead of an empty list.
    #[must_use]
    pub fn empty_message(&self) -> &'static str {
        match self.kind {
            ListKind::History(_) => "No history",
            ListKind::Bookmarks => "No bookmarks",
            ListKind::Drives => "No drives",
            ListKind::Encodings => "No encodings",
            ListKind::Found => "No matches",
        }
    }

    /// Whether the list is narrowed by typing. In bookmarks and drives, typing
    /// picks an item by its key instead.
    #[must_use]
    pub fn filters(&self) -> bool {
        matches!(
            self.kind,
            ListKind::History(_) | ListKind::Encodings | ListKind::Found
        )
    }

    /// The side whose history is listed, if the list is a history.
    #[must_use]
    pub fn history_side(&self) -> Option<Side> {
        match self.kind {
            ListKind::History(side) => Some(side),
            ListKind::Bookmarks | ListKind::Drives | ListKind::Encodings | ListKind::Found => None,
        }
    }

    /// The rows to show: the place, with its key, if it has one, at the end like
    /// the sort popup.
    #[must_use]
    pub fn items(&self) -> Vec<PopupItem> {
        self.matches
            .iter()
            .map(|&index| &self.items[index])
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
        self.matches
            .get(self.cursor)
            .map(|&index| &self.items[index])
    }

    fn history_with(
        side: Side,
        history: &History,
        item: impl Fn(usize, &PathBuf) -> ListItem,
    ) -> Self {
        let items: Vec<ListItem> = history
            .items()
            .iter()
            .enumerate()
            .rev()
            .map(|(pos, path)| item(pos, path))
            .collect();
        Self {
            cursor: history.items().len().saturating_sub(history.pos() + 1),
            ..Self::new(ListKind::History(side), items)
        }
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

impl RenamePopup {
    /// Opens on `targets` of `dir`; `listing` is the pane's whole listing.
    #[must_use]
    pub fn new(
        mode: RenameMode,
        dir: PathBuf,
        targets: Vec<Entry>,
        siblings: bool,
        listing: &[Entry],
        os: Os,
    ) -> Self {
        let fields = match mode {
            RenameMode::Replace => vec![
                InputField {
                    label: "Find",
                    ..InputField::new(String::new())
                },
                InputField {
                    label: "Replace",
                    ..InputField::new(String::new())
                },
            ],
            // Typing replaces the name and keeps the extension, as in `rename`.
            RenameMode::Template => vec![InputField {
                select: Some((0, "{name}".len())),
                ..InputField::new(INITIAL_TEMPLATE.to_owned())
            }],
        };
        let mut popup = Self {
            mode,
            dir,
            targets,
            siblings,
            fields,
            plan: RenamePlan {
                names: Vec::new(),
                error: None,
            },
            top: 0,
            rows: LineRows::default(),
        };
        popup.replan(listing, os);
        popup
    }

    /// Makes the plan again from the fields, against the pane's whole listing.
    pub fn replan(&mut self, listing: &[Entry], os: Os) {
        let text = |field: usize| self.fields.get(field).map_or("", |field| &field.text);
        let input = match self.mode {
            RenameMode::Replace => RenameInput::Replace {
                find: text(0),
                replace: text(1),
            },
            RenameMode::Template => RenameInput::Template(text(0)),
        };
        let existing = self.siblings.then_some(listing);
        let old = std::mem::replace(
            &mut self.plan,
            RenamePlan::new(&self.targets, input, existing, os),
        );
        // The UI measures only the items it is handed anew, so a count kept
        // where another item or name now takes the index could stand for an
        // item that never reports.
        let mut old = renamed(&old.names);
        let mut new = renamed(&self.plan.names);
        let mut index = 0;
        loop {
            let (before, after) = (old.next(), new.next());
            if before.is_none() && after.is_none() {
                break;
            }
            if before != after {
                self.rows.forget(index);
            }
            index += 1;
        }
        // Past the last item nothing is shown or measured, so nothing would
        // bring the top back.
        self.top = self.top.min(self.count().saturating_sub(1));
    }

    pub fn set_text(&mut self, field: usize, text: String) {
        if let Some(field) = self.fields.get_mut(field) {
            field.text = text;
        }
    }

    /// Takes how many rows items take, by index, as the UI measured them.
    pub fn set_line_rows(&mut self, lines: &[(usize, usize)], area: usize) {
        let count = self.count();
        for &(index, rows) in lines {
            if index < count {
                self.rows.set(index, rows);
            }
        }
        self.top = scroll::pull_back(self.top, count, area, |index| self.rows.get(index));
    }

    /// Scrolls by a number of items; positive moves towards the end.
    pub fn scroll_by(&mut self, delta: i64, area: usize) {
        self.top = scroll::move_by(self.top, delta, self.count(), area, |index| {
            self.rows.get(index)
        });
    }

    /// Puts `top` at the top of the list, or the end if it lies beyond.
    pub fn scroll_to(&mut self, top: usize, area: usize) {
        self.top = top.min(self.end(area));
    }

    /// Scrolls for an arrow, page, Home, or End key; other keys do nothing.
    pub fn scroll_key(&mut self, key: KeyBase, area: usize) {
        let count = self.count();
        let rows = |index| self.rows.get(index);
        self.top = match key {
            KeyBase::Named(NamedKey::Up) => scroll::move_by(self.top, -1, count, area, rows),
            KeyBase::Named(NamedKey::Down) => scroll::move_by(self.top, 1, count, area, rows),
            KeyBase::Named(NamedKey::PageUp) => scroll::page_up(self.top, area, rows),
            KeyBase::Named(NamedKey::PageDown) => scroll::page_down(self.top, count, area, rows),
            KeyBase::Named(NamedKey::Home) => 0,
            KeyBase::Named(NamedKey::End) => scroll::end(count, area, rows),
            _ => self.top,
        };
    }

    #[must_use]
    pub fn title(&self) -> &'static str {
        match self.mode {
            RenameMode::Replace => "Rename (replace)",
            RenameMode::Template => "Rename (template)",
        }
    }

    /// The first error, or else the hint about what to type.
    #[must_use]
    pub fn note(&self) -> String {
        self.plan.error.clone().unwrap_or_else(|| {
            match self.mode {
                RenameMode::Replace => REPLACE_HINT,
                RenameMode::Template => TEMPLATE_HINT,
            }
            .to_owned()
        })
    }

    /// The `(old name, new name)` of each target whose name changes, in
    /// display order.
    #[must_use]
    pub fn pairs(&self) -> Vec<(String, String)> {
        renamed(&self.plan.names)
            .map(|(target, new)| {
                (
                    self.targets[target].name.to_string_lossy().into_owned(),
                    new.to_owned(),
                )
            })
            .collect()
    }

    /// The items to lay out or measure with the top at `self.top`: the
    /// targets whose name changes, in display order.
    #[must_use]
    pub fn lines(&self, area: usize) -> PopupLines {
        let count = self.count();
        let ranges = scroll::ranges(self.top, count, area);
        let lines = |range: Range<usize>| {
            renamed(&self.plan.names)
                .enumerate()
                .skip(range.start)
                .take(range.len())
                .map(|(index, (target, new))| PopupLine {
                    index,
                    text: format!(
                        "{} \u{2192} {new}",
                        self.targets[target].name.to_string_lossy()
                    ),
                })
                .collect()
        };
        PopupLines {
            shown: lines(ranges.shown),
            before: lines(ranges.before),
            tail: lines(ranges.tail),
            scroll_top: self.top,
            count,
            end: self.end(area),
        }
    }

    fn count(&self) -> usize {
        renamed(&self.plan.names).count()
    }

    fn end(&self, area: usize) -> usize {
        scroll::end(self.count(), area, |index| self.rows.get(index))
    }
}

/// The part of a file name to select when editing it: everything but the
/// extension, or nothing special when there is none.
#[must_use]
pub fn stem_selection(name: &str) -> Option<(usize, usize)> {
    let stem = ArchiveFormat::stem_of(OsStr::new(name)).len();
    (stem < name.len()).then_some((0, stem))
}

// The popups that pick a row. The rules for keys and the mouse live in
// `menu_input` alone; an implementation only describes its rows and what
// running or confirming one does. Separators are rows here too, so that the
// UI's row numbers are the core's and a click needs no translation.
pub(crate) trait Menu {
    type Pick;

    fn rows(&self) -> usize;
    fn kind(&self, row: usize) -> ItemKind;
    fn cursor(&self) -> usize;
    fn set_cursor(&mut self, row: usize);
    fn row_for_key(&self, key: char) -> Option<usize>;
    fn run(&mut self, row: usize) -> Response<Self::Pick>;

    // Enter on a checkbox. It never flips the box: Enter means "done", so it
    // either confirms what the popup holds or, with nothing to confirm, does
    // nothing.
    fn confirm(&mut self) -> Response<Self::Pick> {
        Response::Stay
    }

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
enum CompareRow {
    Condition(CompareCondition),
    Separator,
    Flag(CompareFlag),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompareFlag {
    IncludeDirs,
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

    fn confirm(&mut self) -> Response<SortSpec> {
        Response::Pick(self.draft)
    }
}

impl Menu for ComparePopup {
    type Pick = CompareSpec;

    fn rows(&self) -> usize {
        COMPARE_ROWS.len()
    }

    fn kind(&self, row: usize) -> ItemKind {
        match COMPARE_ROWS[row].0 {
            CompareRow::Condition(_) => ItemKind::Choice,
            CompareRow::Separator => ItemKind::Separator,
            CompareRow::Flag(flag) => ItemKind::Check(self.flag(flag)),
        }
    }

    fn cursor(&self) -> usize {
        self.cursor
    }

    fn set_cursor(&mut self, row: usize) {
        self.cursor = row;
    }

    fn row_for_key(&self, key: char) -> Option<usize> {
        COMPARE_ROWS.iter().position(|(_, _, row_key)| {
            row_key.is_some_and(|row_key| row_key.eq_ignore_ascii_case(&key))
        })
    }

    fn run(&mut self, row: usize) -> Response<CompareSpec> {
        match COMPARE_ROWS[row].0 {
            CompareRow::Condition(condition) => {
                self.draft.condition = condition;
                Response::Pick(self.draft)
            }
            CompareRow::Separator => Response::Stay,
            CompareRow::Flag(flag) => {
                let flag = self.flag_mut(flag);
                *flag = !*flag;
                Response::Stay
            }
        }
    }

    fn confirm(&mut self) -> Response<CompareSpec> {
        Response::Pick(self.draft)
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

    fn row_for_key(&self, key: char) -> Option<usize> {
        let key = key.to_ascii_lowercase();
        self.matches
            .iter()
            .position(|&index| self.items[index].key == Some(key))
    }

    fn run(&mut self, row: usize) -> Response<ListItem> {
        match self.matches.get(row) {
            Some(&index) => Response::Pick(self.items[index].clone()),
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

pub(crate) fn menu_input<M: Menu>(
    menu: &mut M,
    input: MenuInput,
    page: usize,
) -> Response<M::Pick> {
    match input {
        MenuInput::Key(key) => match key.base {
            KeyBase::Named(NamedKey::Escape) => Response::Close,
            KeyBase::Named(NamedKey::Enter) => match menu.kind(menu.cursor()) {
                ItemKind::Check(_) => menu.confirm(),
                _ => menu.run(menu.cursor()),
            },
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
            KeyBase::Named(NamedKey::PageUp) => {
                move_cursor(menu, menu.cursor().saturating_sub(page), false);
                Response::Stay
            }
            KeyBase::Named(NamedKey::PageDown) => {
                move_cursor(menu, menu.cursor().saturating_add(page), true);
                Response::Stay
            }
            KeyBase::Named(NamedKey::Home) => {
                move_cursor(menu, 0, false);
                Response::Stay
            }
            KeyBase::Named(NamedKey::End) => {
                move_cursor(menu, usize::MAX, true);
                Response::Stay
            }
            KeyBase::Char(c) => match menu.row_for_key(c) {
                Some(row) => menu.run(row),
                None => Response::Stay,
            },
            KeyBase::Named(_) => Response::Stay,
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

fn move_cursor<M: Menu>(menu: &mut M, target: usize, forward: bool) {
    let rows = menu.rows();
    let target = target.min(rows.saturating_sub(1));
    let row = if forward {
        (target..rows)
            .chain((0..target).rev())
            .find(|&row| selectable(menu, row))
    } else {
        (0..=target)
            .rev()
            .chain(target + 1..rows)
            .find(|&row| selectable(menu, row))
    };
    if let Some(row) = row {
        menu.set_cursor(row);
    }
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

fn renamed(names: &[Option<String>]) -> impl Iterator<Item = (usize, &str)> {
    names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| Some((index, name.as_deref()?)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: usize = 3;

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

        assert_eq!(menu_input(&mut popup, key("D"), PAGE), Response::Stay);
        assert_eq!(menu_input(&mut popup, key("G"), PAGE), Response::Stay);
        assert_eq!(menu_input(&mut popup, key("C"), PAGE), Response::Stay);
        let toggled = SortSpec {
            descending: true,
            dirs_first: false,
            ignore_case: false,
            ..SortSpec::default()
        };
        assert_eq!(popup.draft, toggled);
        assert_eq!(
            menu_input(&mut popup, key("E"), PAGE),
            Response::Pick(SortSpec {
                key: SortKey::Extension,
                ..toggled
            })
        );
    }

    #[test]
    fn sort_popup_enter_on_a_toggle_confirms_the_draft_without_flipping_it() {
        let mut popup = SortPopup::new(SortSpec::default());
        popup.cursor = 7;

        assert_eq!(menu_input(&mut popup, key("Space"), PAGE), Response::Stay);
        let toggled = SortSpec {
            descending: true,
            ..SortSpec::default()
        };
        assert_eq!(
            menu_input(&mut popup, key("Enter"), PAGE),
            Response::Pick(toggled)
        );
    }

    #[test]
    fn compare_popup_opens_on_the_remembered_condition() {
        let popup = ComparePopup::new(CompareSpec {
            condition: CompareCondition::Newer,
            include_dirs: false,
            ignore_case: true,
        });

        assert_eq!(popup.cursor, 1);
        let items = popup.items();
        let rows: Vec<(&str, &str, ItemKind)> = items
            .iter()
            .map(|i| (i.label.as_str(), i.detail.as_str(), i.kind))
            .collect();
        assert_eq!(
            rows,
            [
                ("Missing", "M", ItemKind::Choice),
                ("Missing or newer *", "N", ItemKind::Choice),
                ("Missing or different", "D", ItemKind::Choice),
                ("", "", ItemKind::Separator),
                ("Include directories", "I", ItemKind::Check(false)),
                ("Case-insensitive", "C", ItemKind::Check(true)),
            ]
        );
    }

    #[test]
    fn compare_popup_letters_flip_toggles_and_pick_a_condition_with_them() {
        let mut popup = ComparePopup::new(CompareSpec::default());

        assert_eq!(menu_input(&mut popup, key("I"), PAGE), Response::Stay);
        assert_eq!(menu_input(&mut popup, key("C"), PAGE), Response::Stay);
        assert_eq!(
            menu_input(&mut popup, key("D"), PAGE),
            Response::Pick(CompareSpec {
                condition: CompareCondition::Different,
                include_dirs: false,
                ignore_case: false,
            })
        );
    }

    #[test]
    fn password_popup_names_the_archive_and_says_when_the_password_was_wrong() {
        let archive = PathBuf::from("/a/b.zip");

        let first = InputPopup::password(archive.clone(), 0, false, PasswordPurpose::Operation);
        let retry = InputPopup::password(archive, 0, true, PasswordPurpose::Operation);

        assert_eq!(first.message().as_deref(), Some("b.zip"));
        assert_eq!(first.note(), None);
        assert_eq!(retry.message().as_deref(), Some("b.zip"));
        assert_eq!(retry.note().as_deref(), Some("Wrong password"));
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

        assert_eq!(one.message, "a");
        assert_eq!(two.message, "2 items");
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

        assert_eq!(menu_input(&mut popup, key("Down"), PAGE), Response::Stay);
        assert_eq!(popup.cursor, 2);
        assert_eq!(menu_input(&mut popup, key("Up"), PAGE), Response::Stay);
        assert_eq!(popup.cursor, 1);
    }

    #[test]
    fn menu_input_moves_the_cursor_by_a_page_without_wrapping() {
        let mut popup = conflict_popup();

        assert_eq!(
            menu_input(&mut popup, key("PageDown"), PAGE),
            Response::Stay
        );
        assert_eq!(popup.cursor, 4);
        menu_input(&mut popup, key("PageDown"), PAGE);
        assert_eq!(popup.cursor, 6);
        menu_input(&mut popup, key("PageDown"), PAGE);
        assert_eq!(popup.cursor, 6);
        assert_eq!(menu_input(&mut popup, key("PageUp"), PAGE), Response::Stay);
        assert_eq!(popup.cursor, 3);
        menu_input(&mut popup, key("PageUp"), PAGE);
        assert_eq!(popup.cursor, 1);
        menu_input(&mut popup, key("PageUp"), PAGE);
        assert_eq!(popup.cursor, 1);
    }

    #[test]
    fn menu_input_moves_the_cursor_to_the_ends_with_home_and_end() {
        let mut popup = conflict_popup();

        assert_eq!(menu_input(&mut popup, key("End"), PAGE), Response::Stay);
        assert_eq!(popup.cursor, 6);
        assert_eq!(menu_input(&mut popup, key("Home"), PAGE), Response::Stay);
        assert_eq!(popup.cursor, 1);
    }

    #[test]
    fn menu_input_passes_a_separator_in_the_direction_of_a_page_move() {
        let mut popup = SortPopup::new(SortSpec::default());

        popup.cursor = 3;
        menu_input(&mut popup, key("PageDown"), PAGE);
        assert_eq!(popup.cursor, 7);
        popup.cursor = 9;
        menu_input(&mut popup, key("PageUp"), PAGE);
        assert_eq!(popup.cursor, 5);
    }

    #[test]
    fn menu_input_page_keys_leave_an_empty_list_alone() {
        let mut popup = PalettePopup::new(Context::FileList, Os::Linux, Vec::new());
        popup.set_query("no such command".to_owned());

        assert_eq!(
            menu_input(&mut popup, key("PageDown"), PAGE),
            Response::Stay
        );
    }

    #[test]
    fn menu_input_enter_runs_a_choice_row_and_leaves_a_check_row_alone() {
        let mut popup = conflict_popup();

        popup.cursor = 2;
        assert_eq!(
            menu_input(&mut popup, key("Enter"), PAGE),
            Response::Pick(ConflictChoice::Skip)
        );
        popup.cursor = 6;
        assert_eq!(menu_input(&mut popup, key("Enter"), PAGE), Response::Stay);
        assert!(!popup.all);
    }

    #[test]
    fn menu_input_space_toggles_a_check_row_only() {
        let mut popup = conflict_popup();

        assert_eq!(menu_input(&mut popup, key("Space"), PAGE), Response::Stay);
        assert_eq!(popup, conflict_popup());
        popup.cursor = 6;
        assert_eq!(menu_input(&mut popup, key("Space"), PAGE), Response::Stay);
        assert!(popup.all);
    }

    #[test]
    fn menu_input_letters_run_their_row_without_moving_the_cursor() {
        let mut popup = conflict_popup();

        assert_eq!(menu_input(&mut popup, key("A"), PAGE), Response::Stay);
        assert_eq!((popup.cursor, popup.all), (1, true));
        assert_eq!(menu_input(&mut popup, key("X"), PAGE), Response::Stay);
        assert_eq!(
            menu_input(&mut popup, key("O"), PAGE),
            Response::Pick(ConflictChoice::Overwrite)
        );
    }

    #[test]
    fn menu_input_closes_on_escape() {
        let mut popup = conflict_popup();

        assert_eq!(menu_input(&mut popup, key("Esc"), PAGE), Response::Close);
    }

    #[test]
    fn menu_input_clicks_move_the_cursor_and_double_clicks_run_the_row() {
        let mut popup = ConflictPopup::new(PathBuf::from("/a/x"), PathBuf::from("/b/x"), false);

        assert_eq!(
            menu_input(&mut popup, MenuInput::Click { row: 6 }, PAGE),
            Response::Stay
        );
        assert_eq!((popup.cursor, popup.all), (6, false));
        assert_eq!(
            menu_input(&mut popup, MenuInput::Click { row: 5 }, PAGE),
            Response::Stay
        );
        assert_eq!(popup.cursor, 6);
        assert_eq!(
            menu_input(&mut popup, MenuInput::DoubleClick { row: 5 }, PAGE),
            Response::Stay
        );
        assert_eq!(
            menu_input(&mut popup, MenuInput::Click { row: 7 }, PAGE),
            Response::Stay
        );
        assert_eq!(popup.cursor, 6);
        assert_eq!(
            menu_input(&mut popup, MenuInput::DoubleClick { row: 6 }, PAGE),
            Response::Stay
        );
        assert!(popup.all);
        assert_eq!(
            menu_input(&mut popup, MenuInput::DoubleClick { row: 3 }, PAGE),
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
    fn list_popup_of_bookmarks_uses_their_own_keys() {
        let bookmarks = vec![bookmark("/a", None), bookmark("/b", Some('z'))];

        let popup = ListPopup::bookmarks(&bookmarks, Path::new("/home/user"));

        assert_eq!(popup.items()[0].detail, "");
        assert_eq!(popup.items()[1].detail, "Z");
        assert_eq!(popup.row_for_key('Z'), Some(1));
        assert_eq!(popup.row_for_key('0'), None);
    }

    #[test]
    fn list_popup_of_history_lists_the_newest_first() {
        let mut history = History::new();
        for path in ["/a", "/b", "/c"] {
            history.visit(PathBuf::from(path));
        }

        let popup = ListPopup::history(Side::Left, &history, false);

        let item = |pos, path: &str| ListItem {
            key: None,
            label: PathBuf::from(path).display().to_string(),
            current: pos == 2,
            target: ListTarget::HistoryPos(pos, PathBuf::from(path)),
            source: None,
        };
        assert_eq!(popup.items, [item(2, "/c"), item(1, "/b"), item(0, "/a")]);
        assert_eq!(popup.cursor, 0);
    }

    #[test]
    fn list_popup_of_history_inside_an_archive_has_no_current_place() {
        let mut history = History::new();
        history.visit(PathBuf::from("/a"));
        history.visit(PathBuf::from("/b"));
        history.visit(PathBuf::from("/c"));
        history.set_pos(1);

        let popup = ListPopup::history(Side::Left, &history, true);

        assert!(popup.items.iter().all(|item| !item.current));
        assert_eq!(popup.cursor, 1);
    }

    #[test]
    fn list_popup_of_the_other_history_lists_places_to_visit() {
        let mut history = History::new();
        for path in ["/a", "/b", "/c"] {
            history.visit(PathBuf::from(path));
        }
        history.set_pos(1);

        let popup = ListPopup::other_history(Side::Right, &history);

        let item = |path: &str| ListItem {
            key: None,
            label: PathBuf::from(path).display().to_string(),
            current: false,
            target: ListTarget::Path(PathBuf::from(path)),
            source: None,
        };
        assert_eq!(popup.items, [item("/c"), item("/b"), item("/a")]);
        assert_eq!(popup.cursor, 1);
        assert_eq!(popup.title(), "History (right)");
    }

    #[test]
    fn list_popup_of_drives_uses_the_letters_as_keys() {
        let mut popup = ListPopup::drives(&['C', 'D'], Some('D'));

        assert_eq!(popup.items[0].key, Some('c'));
        assert_eq!(popup.items[0].target, ListTarget::Drive('C'));
        assert_eq!(popup.items()[1].label, "D:\\");
        assert_eq!(popup.items()[1].detail, "D");
        assert_eq!(popup.cursor, 1);
        assert!(!popup.filters());
        assert_eq!(
            menu_input(&mut popup, key("C"), PAGE),
            Response::Pick(popup.items[0].clone())
        );
        assert_eq!(popup.empty_message(), "No drives");
    }

    #[test]
    fn list_popup_of_history_narrows_to_the_labels_with_the_query_ignoring_case() {
        let mut popup = history_popup(&["/Downloads", "/docs", "/Music"]);
        popup.cursor = 2;

        popup.set_query("DO".to_owned());

        let labels = |popup: &ListPopup| -> Vec<String> {
            popup.items().into_iter().map(|item| item.label).collect()
        };
        assert_eq!(labels(&popup), [shown("/docs"), shown("/Downloads")]);
        assert_eq!(popup.cursor, 0);
        popup.cursor_up();
        assert_eq!(popup.cursor, 1);
        assert_eq!(popup.selected().unwrap().label, shown("/Downloads"));
        assert_eq!(
            menu_input(&mut popup, key("Enter"), PAGE),
            Response::Pick(popup.items[2].clone())
        );

        popup.set_query("mus".to_owned());
        assert_eq!(labels(&popup), [shown("/Music")]);
        assert_eq!(
            menu_input(&mut popup, MenuInput::DoubleClick { row: 0 }, PAGE),
            Response::Pick(popup.items[0].clone())
        );

        popup.set_query(String::new());
        assert_eq!(popup.items().len(), 3);
    }

    #[test]
    fn list_popup_of_history_matches_a_decomposed_path_with_composed_text() {
        let mut popup = history_popup(&["/アフ\u{309A}リ", "/other"]);

        popup.set_query("アプリ".to_owned());

        assert_eq!(popup.matches, [1]);
    }

    #[test]
    fn list_popup_of_history_without_a_match_has_nothing_to_pick() {
        let mut popup = history_popup(&["/a"]);

        popup.set_query("b".to_owned());
        popup.cursor_down();

        assert_eq!(popup.selected(), None);
        assert_eq!(menu_input(&mut popup, key("Enter"), PAGE), Response::Close);
    }

    #[test]
    fn list_popup_of_encodings_opens_on_the_current_one_after_auto_detect() {
        let popup = ListPopup::encodings("Shift_JIS");

        assert_eq!(popup.items.len(), 1 + ENCODINGS.len());
        assert_eq!(popup.items[0].label, "Auto-detect");
        assert_eq!(popup.items[0].target, ListTarget::Encoding(None));
        assert_eq!(
            popup.selected().unwrap().target,
            ListTarget::Encoding(Some(encoding_rs::SHIFT_JIS))
        );
        assert_eq!(popup.title(), "Encoding");
    }

    #[test]
    fn list_popup_of_encodings_narrows_by_typing() {
        let mut popup = ListPopup::encodings("UTF-8");

        assert!(popup.filters());
        popup.set_query("jp".to_owned());

        let labels: Vec<String> = popup.items().into_iter().map(|item| item.label).collect();
        assert_eq!(labels, ["EUC-JP", "ISO-2022-JP"]);
    }

    #[test]
    fn list_popup_closes_on_enter_when_empty() {
        let mut popup = ListPopup::bookmarks(&[], Path::new("/home/user"));

        assert_eq!(menu_input(&mut popup, key("Enter"), PAGE), Response::Close);
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

    fn history_popup(paths: &[&str]) -> ListPopup {
        let mut history = History::new();
        for path in paths {
            history.visit(PathBuf::from(path));
        }
        ListPopup::history(Side::Left, &history, false)
    }

    fn shown(path: &str) -> String {
        PathBuf::from(path).display().to_string()
    }

    fn bookmark(path: &str, key: Option<char>) -> Bookmark {
        Bookmark {
            name: path.to_owned(),
            path: PathBuf::from(path),
            dir: PathBuf::from("/home/user"),
            key,
        }
    }
}
