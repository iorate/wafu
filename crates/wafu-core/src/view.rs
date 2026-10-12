use std::sync::Arc;

use crate::entry::{Entry, EntryKind};
use crate::pane::Pane;

/// A read-only snapshot of the state for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub title: String,
    pub left: PaneView,
    pub right: PaneView,
    pub log: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneView {
    pub path: String,
    /// Shared with the pane; the same allocation is returned until the listing changes.
    pub rows: Arc<[RowView]>,
    /// The cursor row, present only for the active pane.
    pub cursor: Option<usize>,
    pub scroll_top: usize,
    pub info: String,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowView {
    pub name: String,
    pub extension: String,
    pub size: String,
    pub modified: String,
}

impl PaneView {
    pub(crate) fn from_pane(pane: &Pane, active: bool) -> Self {
        let rows = pane.rows();
        let descending = if pane.sort.descending { " desc" } else { "" };
        let hidden = if pane.show_hidden { "on" } else { "off" };
        Self {
            path: pane.location.to_string(),
            cursor: (active && pane.cursor < rows.len()).then_some(pane.cursor),
            rows,
            scroll_top: pane.scroll_top,
            info: format!(
                "{} items | sort: {}{descending} | hidden: {hidden}",
                pane.listing.visible_len(),
                pane.sort.key.name(),
            ),
            active,
        }
    }
}

impl RowView {
    pub(crate) fn parent() -> Self {
        Self {
            name: "..".to_owned(),
            extension: String::new(),
            size: String::new(),
            modified: String::new(),
        }
    }

    pub(crate) fn from_entry(entry: &Entry) -> Self {
        let (name, extension) = entry.split_name();
        let extension = if extension.is_empty() {
            extension
        } else {
            format!(".{extension}")
        };
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
            name,
            extension,
            size,
            modified,
        }
    }
}
