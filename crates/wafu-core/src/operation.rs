use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use crate::error::IoError;
use crate::location::{ArchiveFormat, ArchivePath};

/// A file operation for the I/O layer to carry out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationSpec {
    pub kind: OperationKind,
    pub sources: Sources,
    /// The destination directory of a copy, move, extraction, or compression.
    pub dest: Option<PathBuf>,
    /// The archive a compression writes.
    pub output: Option<ArchiveOutput>,
}

/// What an operation works on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sources {
    /// Full paths, in display order.
    Paths(Vec<PathBuf>),
    /// Entries of one directory inside an archive.
    Entries(ArchiveEntries),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntries {
    pub archive: PathBuf,
    /// The directory the entries are in.
    pub path: ArchivePath,
    /// In display order.
    pub names: Vec<String>,
    /// The password the pane knows; the worker asks when it needs one and has none.
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveOutput {
    pub format: ArchiveFormat,
    pub name: OsString,
    /// `None` writes without encryption.
    pub password: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Copy,
    Move,
    Delete,
    Trash,
    /// Unpacks whole archives into the destination, each into a directory
    /// named after it when `into_dir`.
    Extract {
        into_dir: bool,
    },
    /// Packs the sources into one archive in the destination.
    Compress,
    /// Unpacks archive entries into a fresh temporary directory, to hand to
    /// another program.
    ExtractToTemp,
}

/// What the I/O layer reports while an operation runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationEvent {
    /// The item being worked on and the number of items finished so far.
    Progress {
        current: PathBuf,
        done: usize,
    },
    /// `sources[index]` was handled completely, with nothing skipped under it.
    ItemDone {
        index: usize,
    },
    /// `dest` exists; the worker waits for a reply.
    Conflict {
        source: PathBuf,
        dest: PathBuf,
    },
    /// An entry of the archive is encrypted; the worker waits for a password.
    /// `retry` means the one it was given was wrong.
    NeedPassword {
        archive: PathBuf,
        retry: bool,
    },
    /// Where an `ExtractToTemp` puts its entries. Reported once, first.
    TempDir(PathBuf),
    Finished(Outcome),
}

/// How an operation ended. The counts are of files, links, and directories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Completed {
        done: usize,
        skipped: usize,
    },
    Cancelled {
        done: usize,
    },
    Failed {
        path: PathBuf,
        error: IoError,
        done: usize,
    },
}

/// The log line for an outcome.
pub struct OutcomeLine<'a> {
    pub kind: OperationKind,
    pub outcome: &'a Outcome,
}

/// The answer to a conflict or a password request, or a cancellation at any time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Overwrite {
        all: bool,
    },
    Skip {
        all: bool,
    },
    /// Another name in the same directory.
    Rename(OsString),
    Password(String),
    Cancel,
}

impl OperationKind {
    /// The verb for the log, in the past tense: `copied`.
    #[must_use]
    pub fn past(self) -> &'static str {
        match self {
            Self::Copy => "copied",
            Self::Move | Self::Trash => "moved",
            Self::Delete => "deleted",
            Self::Extract { .. } | Self::ExtractToTemp => "extracted",
            Self::Compress => "compressed",
        }
    }

    /// The noun for the log: `copy cancelled`.
    #[must_use]
    pub fn noun(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Delete => "delete",
            Self::Trash => "trash",
            Self::Extract { .. } | Self::ExtractToTemp => "extract",
            Self::Compress => "compress",
        }
    }

    /// The title of the progress popup.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Copy => "Copying",
            Self::Move => "Moving",
            Self::Delete => "Deleting",
            Self::Trash => "Moving to trash",
            Self::Extract { .. } | Self::ExtractToTemp => "Extracting",
            Self::Compress => "Compressing",
        }
    }
}

impl fmt::Display for OutcomeLine<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = self.kind;
        match self.outcome {
            Outcome::Completed { done, skipped } => {
                write!(f, "{} {done} {}", kind.past(), items(*done))?;
                if kind == OperationKind::Trash {
                    f.write_str(" to the trash")?;
                }
                if *skipped > 0 {
                    write!(f, " ({skipped} skipped)")?;
                }
                Ok(())
            }
            Outcome::Cancelled { done } => {
                write!(f, "{} cancelled after {done} {}", kind.noun(), items(*done))
            }
            Outcome::Failed { path, error, .. } => {
                write!(f, "cannot {} {}: {error}", kind.noun(), path.display())
            }
        }
    }
}

fn items(count: usize) -> &'static str {
    if count == 1 { "item" } else { "items" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_lines() {
        let line = |kind, outcome: &Outcome| OutcomeLine { kind, outcome }.to_string();

        assert_eq!(
            line(
                OperationKind::Copy,
                &Outcome::Completed {
                    done: 12,
                    skipped: 0
                }
            ),
            "copied 12 items"
        );
        assert_eq!(
            line(
                OperationKind::Move,
                &Outcome::Completed {
                    done: 1,
                    skipped: 2
                }
            ),
            "moved 1 item (2 skipped)"
        );
        assert_eq!(
            line(
                OperationKind::Trash,
                &Outcome::Completed {
                    done: 3,
                    skipped: 0
                }
            ),
            "moved 3 items to the trash"
        );
        assert_eq!(
            line(
                OperationKind::Extract { into_dir: true },
                &Outcome::Completed {
                    done: 4,
                    skipped: 0
                }
            ),
            "extracted 4 items"
        );
        assert_eq!(
            line(OperationKind::Compress, &Outcome::Cancelled { done: 2 }),
            "compress cancelled after 2 items"
        );
        assert_eq!(
            line(OperationKind::Delete, &Outcome::Cancelled { done: 3 }),
            "delete cancelled after 3 items"
        );
        assert_eq!(
            line(
                OperationKind::Copy,
                &Outcome::Failed {
                    path: PathBuf::from("/a/b"),
                    error: IoError::new("permission denied"),
                    done: 0,
                }
            ),
            "cannot copy /a/b: permission denied"
        );
        assert_eq!(
            line(
                OperationKind::ExtractToTemp,
                &Outcome::Failed {
                    path: PathBuf::from("/a/b.zip"),
                    error: IoError::new("bad"),
                    done: 0,
                }
            ),
            "cannot extract /a/b.zip: bad"
        );
    }
}
