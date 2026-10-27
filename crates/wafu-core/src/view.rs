use std::sync::Arc;

use crate::config::{ColorSet, Config, SizeFormat, Theme};
use crate::entry::{Entry, EntryKind};
use crate::pane::Pane;
use crate::preview::ImageData;

/// How many characters of a name stay visible at the end when it is elided in the middle.
/// A read-only snapshot of the state for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub title: String,
    pub left: PaneView,
    pub right: PaneView,
    pub log: LogView,
    pub popup: Option<PopupView>,
    pub preview: Option<PreviewView>,
    /// Whether the log is hidden for the preview.
    pub hide_log: bool,
    /// Width of the left pane as a percentage of the window.
    pub split_percent: u8,
    pub log_rows: usize,
    /// Shared with the application; the same allocation is returned until the
    /// configuration changes.
    pub style: Arc<Style>,
}

/// How the window looks, taken from the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Style {
    pub theme: Theme,
    /// The text the size column is made wide enough for.
    pub size_sample: &'static str,
    pub fonts: Fonts,
    pub colors: ColorSet,
}

/// The fonts of each area, with every fallback already applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fonts {
    pub ui: ResolvedFont,
    pub filelist: ResolvedFont,
    pub log: ResolvedFont,
    pub preview: ResolvedFont,
}

/// A font to use. An empty family means the system font.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedFont {
    pub family: String,
    /// In pixels.
    pub size: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneView {
    pub path: String,
    /// Shared with the pane; the same allocation is returned until the listing changes.
    pub rows: Arc<[RowView]>,
    /// Whether each row is marked. Same sharing as `rows`.
    pub marks: Arc<[bool]>,
    /// The cursor row, present only for the active pane.
    pub cursor: Option<usize>,
    pub scroll_top: usize,
    pub info: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    pub kind: RowKind,
    pub name: String,
    /// The last characters of the name, kept visible when eliding in the middle.
    /// Empty for short names.
    pub extension: String,
    /// `@` for symbolic links, otherwise empty.
    pub marker: String,
    pub size: String,
    pub modified: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogView {
    /// Shared with the log; the same allocation is returned until the lines change.
    pub lines: Arc<[String]>,
    pub scroll_top: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PopupView {
    pub title: String,
    /// A line shown under the title.
    pub message: Option<String>,
    /// The input fields, if the popup has any. The first one takes the focus.
    pub fields: Vec<InputField>,
    pub items: Vec<PopupItem>,
    /// The selected item, if the popup has a list.
    pub cursor: Option<usize>,
    /// Reports work going on in the background; the UI may delay showing it.
    pub progress: bool,
}

/// One input field of a popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputField {
    /// Shown beside the field when the popup has more than one.
    pub label: &'static str,
    pub text: String,
    /// Hides what is typed, as for a password.
    pub secret: bool,
    /// The part of the text to select when the field opens, as byte offsets.
    /// The whole text is selected when this is `None`.
    pub select: Option<(usize, usize)>,
}

impl InputField {
    #[must_use]
    pub fn new(text: String) -> Self {
        Self {
            label: "",
            text,
            secret: false,
            select: None,
        }
    }
}

/// An open preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewView {
    pub placement: PreviewPlacement,
    pub content: PreviewContent,
    /// The file's name, for the overlay in the corner.
    pub name: String,
    /// What the overlay says beside the name: the encoding and kind of a
    /// document, or whether the image's entry is marked.
    pub detail: String,
}

/// Where a preview is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewPlacement {
    /// In place of both panes.
    Full,
    /// In place of the right pane, with the left pane still shown.
    RightPane,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewContent {
    /// The visible lines of a text or binary preview, from the scroll
    /// position. Empty while the file is being read. `digits` is the width
    /// of the line number column; zero when there are no numbers.
    Lines {
        lines: Vec<PreviewLine>,
        digits: usize,
    },
    /// An image, or nothing when the cursor is not on an image.
    Image(Option<SharedImage>),
    /// Why an image could not be shown.
    Message(String),
}

/// One visible line of a document preview.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewLine {
    /// The line number, or empty for a continued line and for the hex dump.
    pub number: String,
    pub text: String,
}

/// An image shared with the application. Equality is by identity, so that
/// comparing two views never compares pixels.
#[derive(Debug, Clone)]
pub struct SharedImage(pub Arc<ImageData>);

impl PartialEq for SharedImage {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SharedImage {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupItem {
    pub label: String,
    pub detail: String,
    pub mark: ItemMark,
}

/// The indicator drawn before a popup item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemMark {
    None,
    Radio(bool),
    Check(bool),
}

/// What a row shows, for its color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Parent,
    Dir,
    File,
    /// Any symbolic link, whatever it points to.
    Link,
}

impl Style {
    pub(crate) fn from_config(config: &Config) -> Self {
        let font = &config.appearance.font;
        let resolve = |area: &crate::config::Font| {
            let (family, size) = area.resolve(&font.default);
            ResolvedFont { family, size }
        };
        Self {
            theme: config.appearance.theme,
            size_sample: config.appearance.size_format.column_sample(),
            fonts: Fonts {
                ui: resolve(&font.default),
                filelist: resolve(&font.filelist),
                log: resolve(&font.log),
                preview: resolve(&font.preview),
            },
            colors: config.colors.clone(),
        }
    }
}

impl PaneView {
    pub(crate) fn from_pane(pane: &Pane, active: bool) -> Self {
        let rows = pane.rows();
        let (marked, bytes) = pane.marked_summary();
        let bytes = match pane.size_format() {
            SizeFormat::Human => SizeFormat::Human.format(bytes),
            format => format!("{} bytes", format.format(bytes)),
        };
        let filter = pane
            .filter()
            .map(|filter| format!(" | filter: {filter}"))
            .unwrap_or_default();
        let descending = if pane.sort().descending { " desc" } else { "" };
        let hidden = if pane.show_hidden() { "on" } else { "off" };
        Self {
            path: pane.location().to_string(),
            cursor: (active && pane.cursor() < rows.len()).then_some(pane.cursor()),
            rows,
            marks: pane.mark_flags(),
            scroll_top: pane.scroll_top(),
            info: format!(
                "{} items | {marked} marked ({bytes}){filter} | sort: {}{descending} | hidden: {hidden}",
                pane.listing().visible_len(),
                pane.sort().key.name(),
            ),
            active,
        }
    }
}

impl RowView {
    pub(crate) fn parent() -> Self {
        Self {
            kind: RowKind::Parent,
            name: "..".to_owned(),
            extension: String::new(),
            marker: String::new(),
            size: String::new(),
            modified: String::new(),
        }
    }

    pub(crate) fn from_entry(entry: &Entry, size_format: SizeFormat) -> Self {
        let (name, extension) = entry.split_name();
        let extension = if extension.is_empty() {
            extension
        } else {
            format!(".{extension}")
        };
        let kind = match entry.kind {
            EntryKind::Dir => RowKind::Dir,
            EntryKind::File => RowKind::File,
            EntryKind::Symlink { .. } => RowKind::Link,
        };
        let is_link = kind == RowKind::Link;
        let broken = entry.kind == EntryKind::Symlink { target: None };
        let size = if broken {
            "?".to_owned()
        } else if entry.kind.is_dir_like() {
            "<DIR>".to_owned()
        } else {
            size_format.format(entry.size)
        };
        let modified = if broken {
            "?".to_owned()
        } else {
            entry
                .modified
                .map(|t| t.local.to_string())
                .unwrap_or_default()
        };
        Self {
            kind,
            name,
            extension,
            marker: if is_link {
                "@".to_owned()
            } else {
                String::new()
            },
            size,
            modified,
        }
    }
}
