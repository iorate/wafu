use std::fmt;
use std::path::PathBuf;

use thiserror::Error;

use crate::app::SpawnName;
use crate::command::Command;
use crate::external::CommandLineError;
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
    #[error("Cannot read {}: {source}", path.display())]
    ReadDir { path: PathBuf, source: IoError },
    #[error("Cannot watch {}: {source}", path.display())]
    Watch { path: PathBuf, source: IoError },
    #[error("Cannot use pattern \"{pattern}\": {source}")]
    InvalidPattern {
        pattern: String,
        source: FilterError,
    },
    #[error("Cannot open {}: the symbolic link is broken", path.display())]
    BrokenLink { path: PathBuf },
    #[error("Cannot open {}: not a regular file", path.display())]
    NotRegularFile { path: PathBuf },
    #[error("Cannot open {}: not available; opened {}", path.display(), fallback.display())]
    Fallback { path: PathBuf, fallback: PathBuf },
    #[error("Cannot open {}: not available", path.display())]
    Unavailable { path: PathBuf },
    #[error("Cannot open {}: does not exist", path.display())]
    NotFound { path: PathBuf },
    #[error("Cannot run {command}: available only on Windows")]
    WindowsOnly { command: Command },
    #[error("Cannot run {command}: not available inside an archive")]
    InArchive { command: Command },
    #[error("Cannot run {command}: no marks")]
    NoMarks { command: Command },
    #[error("Cannot {verb}: the other pane is inside an archive")]
    IntoArchive { verb: &'static str },
    #[error("Cannot open {}: nested archives are not supported", path.display())]
    NestedArchive { path: PathBuf },
    #[error("Cannot compress to {}: only zip takes a password", path.display())]
    NoPassword { path: PathBuf },
    #[error("Cannot compress to {}: unknown archive format", path.display())]
    NoFormat { path: PathBuf },
    #[error("Config: {}:{line}: {message}", path.display())]
    ConfigSyntax {
        path: PathBuf,
        line: usize,
        message: String,
    },
    #[error("Cannot read {}: {source}", path.display())]
    ConfigUnreadable { path: PathBuf, source: IoError },
    #[error("Cannot read {}: circular extends", path.display())]
    ConfigCycle { path: PathBuf },
    #[error("Cannot read {}: {source}", path.display())]
    StateUnreadable { path: PathBuf, source: StateError },
    #[error("Cannot run {command}: unknown command")]
    UnknownCommand { command: Command },
    #[error("Cannot run {command}: {source}")]
    InvalidCommandLine {
        command: Command,
        source: CommandLineError,
    },
    #[error("Cannot start {program}: {source}")]
    SpawnFailed { program: String, source: IoError },
    #[error("Cannot read the login shell's environment: {source}")]
    ShellEnvFailed { source: IoError },
    #[error("Cannot start a new window{}: {source}", if *as_admin { " as administrator" } else { "" })]
    NewWindowFailed { as_admin: bool, source: IoError },
    #[error("{} exited with status {code}", Ended(name))]
    ExitedWithStatus { name: SpawnName, code: i32 },
    #[error("{} was terminated by a signal", Ended(name))]
    TerminatedBySignal { name: SpawnName },
    #[error("Cannot {verb}: the other pane has no location")]
    NoOtherLocation { verb: &'static str },
    #[error("Cannot rename {}: {source}", path.display())]
    RenameFailed { path: PathBuf, source: IoError },
    #[error("Cannot create {}: {source}", path.display())]
    CreateFailed { path: PathBuf, source: IoError },
    #[error("Cannot copy to the clipboard: {source}")]
    ClipboardFailed { source: IoError },
    #[error("Cannot open {}: {source}", path.display())]
    OpenFailed { path: PathBuf, source: IoError },
    #[error("Cannot preview {}: {source}", path.display())]
    PreviewFailed { path: PathBuf, source: IoError },
    #[error("Cannot show the shell menu: {source}")]
    ShellMenuFailed { source: IoError },
}

struct Ended<'a>(&'a SpawnName);

impl fmt::Display for Ended<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            SpawnName::Editor => f.write_str("Editor"),
            SpawnName::Command(command) => write!(f, "Command {command}"),
        }
    }
}
