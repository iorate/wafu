//! Pure model of the wafu file manager.
//!
//! The application state lives in [`App`]. Everything that happens is fed in as
//! a [`Msg`]; everything the application wants done to the outside world comes
//! back out as an [`Effect`]. This crate performs no I/O of its own.

#![forbid(unsafe_code)]

mod app;
mod command;
mod entry;
mod error;
mod filter;
mod history;
mod key;
mod keymap;
mod listing;
mod location;
mod log;
mod pane;
mod path;
mod popup;
#[cfg(test)]
mod test_support;
mod text;
mod view;

pub use app::{App, DrivePurpose, Effect, IoEvent, MouseInput, Msg, Os, Resolved, Side};
pub use command::{Command, Context};
pub use entry::{Entry, EntryKind, LocalDateTime, TargetKind, Timestamp};
pub use error::{AppError, IoError};
pub use filter::{Filter, FilterError};
pub use history::History;
pub use key::{Key, KeyBase, KeyInput, KeyParseError, NamedKey};
pub use keymap::KeyMap;
pub use listing::{Listing, SortKey, SortSpec};
pub use location::{ArchivePath, Location};
pub use log::Log;
pub use pane::{HistoryMove, InFlight, Pane, RequestId, TargetRule};
pub use view::{Ellipsis, ItemMark, LogView, PaneView, PopupItem, PopupView, RowView, View};
