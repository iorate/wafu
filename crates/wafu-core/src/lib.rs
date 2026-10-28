//! Pure model of the wafu file manager.
//!
//! The application state lives in [`App`]. Everything that happens is fed in as
//! a [`Msg`]; everything the application wants done to the outside world comes
//! back out as an [`Effect`]. This crate performs no I/O of its own.

#![forbid(unsafe_code)]

mod app;
mod command;
mod config;
mod entry;
mod error;
mod external;
mod filter;
mod history;
mod key;
mod keymap;
mod listing;
mod location;
mod log;
mod operation;
mod pane;
mod path;
mod popup;
mod preview;
mod state;
#[cfg(test)]
mod test_support;
mod text;
mod view;

pub use app::{
    App, Change, ConfigSource, DrivePurpose, Effect, IoEvent, MenuAnchor, MouseInput, Msg, Os,
    PreviewError, PreviewSource, Resolved, Side, SpawnKind, Start,
};
pub use command::{Command, Context};
pub use config::{
    Appearance, CONFIG_TEMPLATE, Color, ColorSet, Colors, Config, Diagnostic, DiagnosticKind, Font,
    FontSet, General, Initial, SyntaxError, Theme, parse_config,
};
pub use entry::{
    Directory, Entry, EntryKind, LocalDateTime, Permissions, SpecialKind, TargetKind, Timestamp,
};
pub use error::{AppError, IoError};
pub use external::{CommandLine, CommandLineError, Expansion, Placeholder};
pub use filter::{Filter, FilterError};
pub use history::History;
pub use key::{Key, KeyBase, KeyInput, KeyParseError, NamedKey};
pub use keymap::KeyMap;
pub use listing::{Listing, SortKey, SortSpec};
pub use location::{ArchiveFormat, ArchivePath, Location};
pub use log::Log;
pub use operation::{
    ArchiveEntries, ArchiveOutput, OperationEvent, OperationKind, OperationSpec, Outcome,
    OutcomeLine, Reply, Sources,
};
pub use pane::{HistoryMove, InFlight, Pane, RequestId, TargetRule};
pub use preview::{
    Document, DocumentMode, ImageData, Line, PREVIEW_LIMIT, TRUNCATED_NOTE, is_image_name,
};
pub use state::{PaneState, StateError, StateFile, WindowState};
pub use view::{
    Fonts, InputField, ItemMark, LogView, PaneView, PopupItem, PopupView, PreviewContent,
    PreviewLine, PreviewPlacement, PreviewView, ResolvedFont, RowKind, RowView, SharedImage, Style,
    View,
};
