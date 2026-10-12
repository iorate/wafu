use std::path::PathBuf;

use thiserror::Error;

use crate::filter::FilterError;

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
    #[error("{} is not available; moved to {}", path.display(), fallback.display())]
    Fallback { path: PathBuf, fallback: PathBuf },
    #[error("{command} is available only on Windows")]
    WindowsOnly { command: &'static str },
}
