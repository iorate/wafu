use std::sync::Arc;

use crate::app::Os;
use crate::config::{ColorSet, Config, SizeFormat, Theme, monospace_family};
use crate::entry::{Entry, EntryKind, SpecialKind, TargetKind};
use crate::pane::Pane;
use crate::preview::ImageData;

/// A read-only snapshot of the state for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub title: String,
    pub left: PaneView,
    pub right: PaneView,
    pub log: LogView,
    /// The cursor entry of the active pane, spelled out under the panes.
    pub status: String,
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
    pub path: ResolvedFont,
    pub filelist: ResolvedFont,
    pub info: ResolvedFont,
    pub log: ResolvedFont,
    pub status: ResolvedFont,
    pub preview: ResolvedFont,
    pub popup: ResolvedFont,
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
    /// The extension with its leading dot, or empty.
    pub extension: String,
    /// The mark `ls -F` prints after the name (`/`, `*`, `@`, `|`, `=`), or empty.
    pub marker: &'static str,
    pub size: String,
    pub modified: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogView {
    /// Shared with the log; the same allocation is returned until the lines change.
    pub lines: Arc<[Arc<str>]>,
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
    /// What the overlay in the corner says: the encoding of a text document,
    /// or the format of the image and whether its entry is marked. Empty when
    /// there is nothing to say.
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

/// What a row shows, for its color. Each variant takes the color of the same
/// name, except `File`, which takes the text color.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Dir,
    File,
    /// An executable file.
    Exec,
    /// A symbolic link that can be followed, whatever it points to.
    Link,
    /// A broken symbolic link.
    Broken,
    /// A FIFO or a device.
    Fifo,
    Socket,
}

impl Style {
    pub(crate) fn from_config(config: &Config, os: Os) -> Self {
        let font = &config.appearance.font;
        let monospace = monospace_family(os);
        let resolve = |area: &crate::config::Font, family: &str| {
            let (family, size) = area.resolve(&font.default, family);
            ResolvedFont { family, size }
        };
        Self {
            theme: config.appearance.theme,
            size_sample: config.appearance.size_format.column_sample(),
            fonts: Fonts {
                path: resolve(&font.path, monospace),
                filelist: resolve(&font.filelist, monospace),
                info: resolve(&font.info, monospace),
                log: resolve(&font.log, monospace),
                status: resolve(&font.status, monospace),
                preview: resolve(&font.preview, monospace),
                popup: resolve(&font.popup, ""),
            },
            colors: config.colors.clone(),
        }
    }
}

impl PaneView {
    pub(crate) fn from_pane(pane: &Pane, active: bool) -> Self {
        let rows = pane.rows();
        let (marked, bytes) = pane.marked_summary();
        let bytes = size_with_unit(pane.size_format(), bytes);
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
    // The `..` row, whose marker, size, and time columns and color describe the location itself.
    pub(crate) fn parent(own: Option<&Entry>, size_format: SizeFormat) -> Self {
        let (size, modified) =
            own.map_or_else(Default::default, |own| size_and_modified(own, size_format));
        Self {
            kind: own.map_or(RowKind::File, |own| RowKind::from_entry_kind(own.kind)),
            name: "..".to_owned(),
            extension: String::new(),
            marker: own.map_or("", |own| own.kind.marker()),
            size,
            modified,
        }
    }

    pub(crate) fn from_entry(entry: &Entry, size_format: SizeFormat) -> Self {
        let (name, extension) = entry.split_name();
        let extension = if extension.is_empty() {
            extension
        } else {
            format!(".{extension}")
        };
        let (size, modified) = size_and_modified(entry, size_format);
        Self {
            kind: RowKind::from_entry_kind(entry.kind),
            name,
            extension,
            marker: entry.kind.marker(),
            size,
            modified,
        }
    }
}

impl RowKind {
    fn from_entry_kind(kind: EntryKind) -> Self {
        match kind {
            EntryKind::Dir => Self::Dir,
            EntryKind::File { executable: false } => Self::File,
            EntryKind::File { executable: true } => Self::Exec,
            EntryKind::Symlink { target: Some(_) } => Self::Link,
            EntryKind::Symlink { target: None } => Self::Broken,
            EntryKind::Special(SpecialKind::Socket) => Self::Socket,
            EntryKind::Special(_) => Self::Fifo,
        }
    }
}

fn size_and_modified(entry: &Entry, size_format: SizeFormat) -> (String, String) {
    if entry.kind.is_broken_link() {
        return ("?".to_owned(), "?".to_owned());
    }
    let size = if entry.kind.is_dir_like() || entry.kind.is_special() {
        String::new()
    } else {
        size_format.format(entry.size)
    };
    let modified = entry
        .modified
        .map(|t| t.local.to_string())
        .unwrap_or_default();
    (size, modified)
}

pub(crate) fn status_line(pane: &Pane) -> String {
    if let Some(entry) = pane.cursor_entry() {
        let name = entry.name.to_string_lossy().into_owned();
        return entry_status(entry, name, pane.size_format());
    }
    if !pane.has_parent() || pane.cursor() != 0 {
        return String::new();
    }
    match pane.listing().own() {
        Some(own) => entry_status(own, "..".to_owned(), pane.size_format()),
        None => "..".to_owned(),
    }
}

fn entry_status(entry: &Entry, mut name: String, size_format: SizeFormat) -> String {
    match entry.kind {
        EntryKind::Symlink { target } => {
            if let Some(link_target) = &entry.link_target {
                let marker = target.map_or("", TargetKind::marker);
                name = format!("{name} -> {}{marker}", link_target.to_string_lossy());
            }
        }
        kind => name.push_str(kind.marker()),
    }
    let mut segments = vec![name, entry.permissions_text()];
    if entry.kind.is_broken_link() {
        segments.push("?".to_owned());
        segments.push("?".to_owned());
    } else {
        if !entry.kind.is_dir_like() && !entry.kind.is_special() {
            segments.push(size_with_unit(size_format, entry.size));
        }
        if let Some(modified) = entry.modified {
            segments.push(modified.local.to_string());
        }
    }
    segments.join(" | ")
}

fn size_with_unit(format: SizeFormat, bytes: u64) -> String {
    match format {
        SizeFormat::Human => SizeFormat::Human.format(bytes),
        format => format!("{} bytes", format.format(bytes)),
    }
}
