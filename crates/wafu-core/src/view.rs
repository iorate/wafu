use std::sync::Arc;

use crate::config::{ColorSet, Config, Theme};
use crate::entry::{Entry, EntryKind};
use crate::pane::Pane;

/// How many characters of a name stay visible at the end when it is elided in the middle.
const TAIL_CHARS: usize = 8;

/// A read-only snapshot of the state for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub title: String,
    pub left: PaneView,
    pub right: PaneView,
    pub log: LogView,
    pub popup: Option<PopupView>,
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
    pub ellipsis: Ellipsis,
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
    /// The name without its last characters, which are in `name_tail`.
    pub name: String,
    /// The last characters of the name, kept visible when eliding in the middle.
    /// Empty for short names.
    pub name_tail: String,
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
    /// The text of the input field, if the popup has one.
    pub input: Option<String>,
    pub items: Vec<PopupItem>,
    /// The selected item, if the popup has a list.
    pub cursor: Option<usize>,
    /// Reports work going on in the background; the UI may delay showing it.
    pub progress: bool,
}

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

/// Where a name is cut when it does not fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ellipsis {
    End,
    Middle,
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
            ellipsis: config.appearance.ellipsis,
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
                "{} items | {marked} marked ({bytes} bytes){filter} | sort: {}{descending} | hidden: {hidden}",
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
            name_tail: String::new(),
            extension: String::new(),
            marker: String::new(),
            size: String::new(),
            modified: String::new(),
        }
    }

    pub(crate) fn from_entry(entry: &Entry) -> Self {
        let (name, extension) = entry.split_name();
        let (name, name_tail) = split_tail(name);
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
            entry.size.to_string()
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
            name_tail,
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

fn split_tail(mut name: String) -> (String, String) {
    let count = name.chars().count();
    if count <= TAIL_CHARS {
        return (name, String::new());
    }
    let at = name
        .char_indices()
        .nth(count - TAIL_CHARS)
        .map_or(name.len(), |(index, _)| index);
    let tail = name.split_off(at);
    (name, tail)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_have_no_tail() {
        assert_eq!(
            split_tail("12345678".to_owned()),
            ("12345678".to_owned(), String::new())
        );
    }

    #[test]
    fn long_names_keep_the_last_eight_characters_in_the_tail() {
        assert_eq!(
            split_tail("abcdefghij".to_owned()),
            ("ab".to_owned(), "cdefghij".to_owned())
        );
        assert_eq!(
            split_tail("日本語のファイル名です".to_owned()),
            ("日本語".to_owned(), "のファイル名です".to_owned())
        );
    }
}
