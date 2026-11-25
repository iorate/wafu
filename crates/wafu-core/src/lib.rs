//! Pure model of the wafu file manager.
//!
//! The application state lives in [`App`]. Everything that happens is fed in as
//! a [`Msg`]; everything the application wants done to the outside world comes
//! back out as an [`Effect`]. This crate performs no I/O of its own.

#![forbid(unsafe_code)]

mod app;
mod batch_rename;
mod command;
mod compare;
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
mod scroll;
mod state;
#[cfg(test)]
mod test_support;
mod text;
mod view;

pub use app::{
    App, Change, ColorScheme, ConfigContent, ConfigFile, ConfigSource, Effect, IoEvent, MenuAnchor,
    MouseInput, Msg, Os, PreviewSource, ReadError, Resolved, Side, SpawnKind, SpawnName, Start,
};
pub use command::Command;
pub use config::{
    CONFIG_TEMPLATE, Color, Colors, SizeFormat, Theme, byte_count, read_config_chain,
};
pub use entry::{
    Directory, Entry, EntryKind, LocalDateTime, Permissions, SpecialKind, TargetKind, Timestamp,
};
pub use error::IoError;
pub use filter::Filter;
pub use key::KeyInput;
pub use location::{ArchiveFormat, ArchivePath, Location, OuterArchive, Passwords};
pub use operation::{
    ArchiveEntries, ArchiveOutput, FindQuery, OperationEvent, OperationKind, OperationSpec,
    Outcome, Renames, Reply, Sources,
};
pub use pane::{DrivePurpose, RequestId};
pub use preview::{Document, DocumentMode, ImageData, Line};
pub use state::{CompareState, PaneState, ScreenRect, StateError, StateFile, WindowState};
pub use text::nfc;
pub use view::{
    ColorSet, ExtensionColor, Fonts, InputField, ItemKind, LineRanges, LogView, PaneView,
    PopupItem, PopupLine, PopupLines, PopupView, PreviewContent, PreviewLine, PreviewPlacement,
    PreviewView, ResolvedFont, RowKind, RowView, SharedImage, Style, View,
};
