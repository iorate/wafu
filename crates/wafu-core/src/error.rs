use std::path::PathBuf;

use thiserror::Error;

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
}
