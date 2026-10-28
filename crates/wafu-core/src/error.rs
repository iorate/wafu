use std::path::PathBuf;

use thiserror::Error;

use crate::command::Command;
use crate::filter::FilterError;
use crate::state::StateError;

/// An error reported by the I/O layer, flattened to a message.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct IoError {
    message: String,
}

impl IoError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// An error shown to the user in the log.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AppError {
    #[error("cannot read {}: {source}", path.display())]
    ReadDir { path: PathBuf, source: IoError },
    #[error("cannot watch {}: {source}", path.display())]
    Watch { path: PathBuf, source: IoError },
    #[error("invalid pattern {pattern}: {source}")]
    InvalidPattern {
        pattern: String,
        source: FilterError,
    },
    #[error("cannot open {}: the symbolic link is broken", path.display())]
    BrokenLink { path: PathBuf },
    #[error("cannot open {}: not a regular file", path.display())]
    NotRegularFile { path: PathBuf },
    #[error("{} is not available; moved to {}", path.display(), fallback.display())]
    Fallback { path: PathBuf, fallback: PathBuf },
    #[error("{} is not available", path.display())]
    Unavailable { path: PathBuf },
    #[error("{} does not exist", path.display())]
    NotFound { path: PathBuf },
    #[error("{command} is available only on Windows")]
    WindowsOnly { command: Command },
    #[error("{command} is not available inside an archive")]
    InArchive { command: Command },
    #[error("no marks for {command}")]
    NoMarks { command: Command },
    #[error("cannot {verb} into an archive")]
    IntoArchive { verb: &'static str },
    #[error("cannot open {}: nested archives are not supported", path.display())]
    NestedArchive { path: PathBuf },
    #[error("config: {}:{line}: {message}", path.display())]
    ConfigSyntax {
        path: PathBuf,
        line: usize,
        message: String,
    },
    #[error("cannot read {}: {source}", path.display())]
    ConfigUnreadable { path: PathBuf, source: IoError },
    #[error("cannot read the state file: {source}")]
    StateUnreadable { source: StateError },
    #[error("unknown command {command}")]
    UnknownCommand { command: Command },
    #[error("cannot start {program}: {source}")]
    SpawnFailed { program: String, source: IoError },
    #[error("cannot start a new window: {source}")]
    NewWindowFailed { source: IoError },
    #[error("cannot {verb}: the other pane has no location")]
    NoOtherLocation { verb: &'static str },
    #[error("cannot rename {}: {source}", path.display())]
    RenameFailed { path: PathBuf, source: IoError },
    #[error("cannot create {}: {source}", path.display())]
    CreateFailed { path: PathBuf, source: IoError },
    #[error("cannot copy to the clipboard: {source}")]
    ClipboardFailed { source: IoError },
    #[error("cannot open {}: {source}", path.display())]
    OpenFailed { path: PathBuf, source: IoError },
    #[error("cannot preview {}: {source}", path.display())]
    PreviewFailed { path: PathBuf, source: IoError },
    #[error("cannot show the shell menu: {source}")]
    ShellMenuFailed { source: IoError },
}
