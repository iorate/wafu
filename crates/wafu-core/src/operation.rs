use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use crate::config::SizeFormat;
use crate::error::IoError;
use crate::location::{ArchiveFormat, ArchivePath};
use crate::view::size_with_unit;

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
    /// Adds up the sizes of the files in the sources, without following links.
    CalculateSize,
    /// Adds up the space the files in the sources take up on disk, without
    /// following links.
    CalculateSizeOnDisk,
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
    /// One entry could not be written and was left out; the operation goes on.
    Skipped {
        path: PathBuf,
        error: IoError,
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

/// How an operation ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// `skipped` counts the files, links, and directories left out.
    Completed {
        skipped: usize,
    },
    Cancelled,
    /// The end of a `CalculateSize` or a `CalculateSizeOnDisk`. Links count
    /// as nothing, and `unreadable` entries were left out of the counts with
    /// what is under them.
    Measured {
        files: usize,
        directories: usize,
        bytes: u64,
        unreadable: usize,
    },
    Failed {
        path: PathBuf,
        error: IoError,
    },
}

/// The log line for an outcome.
pub struct OutcomeLine<'a> {
    pub kind: OperationKind,
    pub outcome: &'a Outcome,
    /// The names of what the operation was started on.
    pub targets: &'a [OsString],
    pub size_format: SizeFormat,
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
    /// The verb for a line saying the operation cannot be done: `Cannot copy`.
    #[must_use]
    pub fn verb(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Move => "move",
            Self::Delete => "delete",
            Self::Trash => "trash",
            Self::Extract { .. } | Self::ExtractToTemp => "extract",
            Self::Compress => "compress",
            Self::CalculateSize | Self::CalculateSizeOnDisk => "calculate size of",
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
            Self::CalculateSize => "Calculating size",
            Self::CalculateSizeOnDisk => "Calculating size on disk",
        }
    }

    fn past(self) -> &'static str {
        match self {
            Self::Copy => "Copied",
            Self::Move | Self::Trash => "Moved",
            Self::Delete => "Deleted",
            Self::Extract { .. } | Self::ExtractToTemp => "Extracted",
            Self::Compress => "Compressed",
            Self::CalculateSize | Self::CalculateSizeOnDisk => "Calculated",
        }
    }

    fn noun(self) -> &'static str {
        match self {
            Self::Copy => "Copy",
            Self::Move => "Move",
            Self::Delete => "Delete",
            Self::Trash => "Trash",
            Self::Extract { .. } | Self::ExtractToTemp => "Extract",
            Self::Compress => "Compress",
            Self::CalculateSize | Self::CalculateSizeOnDisk => "Size calculation",
        }
    }
}

impl fmt::Display for OutcomeLine<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = self.kind;
        match self.outcome {
            Outcome::Completed { skipped } => {
                write!(f, "{} {}", kind.past(), Subject(self.targets))?;
                if kind == OperationKind::Trash {
                    f.write_str(" to the trash")?;
                }
                if *skipped > 0 {
                    write!(f, " ({skipped} skipped)")?;
                }
                Ok(())
            }
            Outcome::Cancelled => write!(f, "{} cancelled", kind.noun()),
            Outcome::Measured {
                files,
                directories,
                bytes,
                unreadable,
            } => {
                let size = if kind == OperationKind::CalculateSizeOnDisk {
                    "Size on disk"
                } else {
                    "Size"
                };
                write!(
                    f,
                    "{size} of {}: {} ({files} {}, {directories} {}",
                    Subject(self.targets),
                    size_with_unit(self.size_format, *bytes),
                    plural(*files, "file", "files"),
                    plural(*directories, "directory", "directories"),
                )?;
                if *unreadable > 0 {
                    write!(f, ", {unreadable} unreadable")?;
                }
                f.write_str(")")
            }
            Outcome::Failed { path, error } => {
                write!(f, "Cannot {} {}: {error}", kind.verb(), path.display())
            }
        }
    }
}

struct Subject<'a>(&'a [OsString]);

impl fmt::Display for Subject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            [name] => write!(f, "{}", name.to_string_lossy()),
            targets => write!(f, "{} items", targets.len()),
        }
    }
}

fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
    if count == 1 { one } else { many }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcome_lines() {
        let one = [OsString::from("docs")];
        let three = [
            OsString::from("a"),
            OsString::from("b"),
            OsString::from("c"),
        ];
        let line = |kind, outcome: &Outcome, targets: &[OsString]| {
            OutcomeLine {
                kind,
                outcome,
                targets,
                size_format: SizeFormat::Plain,
            }
            .to_string()
        };

        assert_eq!(
            line(
                OperationKind::Copy,
                &Outcome::Completed { skipped: 0 },
                &one
            ),
            "Copied docs"
        );
        assert_eq!(
            line(
                OperationKind::Copy,
                &Outcome::Completed { skipped: 0 },
                &three
            ),
            "Copied 3 items"
        );
        assert_eq!(
            line(
                OperationKind::Move,
                &Outcome::Completed { skipped: 2 },
                &one
            ),
            "Moved docs (2 skipped)"
        );
        assert_eq!(
            line(
                OperationKind::Copy,
                &Outcome::Completed { skipped: 3 },
                &three
            ),
            "Copied 3 items (3 skipped)"
        );
        assert_eq!(
            line(
                OperationKind::Trash,
                &Outcome::Completed { skipped: 0 },
                &one
            ),
            "Moved docs to the trash"
        );
        assert_eq!(
            line(OperationKind::Compress, &Outcome::Cancelled, &three),
            "Compress cancelled"
        );
        assert_eq!(
            line(
                OperationKind::CalculateSizeOnDisk,
                &Outcome::Cancelled,
                &one
            ),
            "Size calculation cancelled"
        );
        assert_eq!(
            line(
                OperationKind::Copy,
                &Outcome::Failed {
                    path: PathBuf::from("/a/b"),
                    error: IoError::new("permission denied"),
                },
                &three
            ),
            "Cannot copy /a/b: permission denied"
        );
        assert_eq!(
            line(
                OperationKind::CalculateSize,
                &Outcome::Failed {
                    path: PathBuf::from("/a/b"),
                    error: IoError::new("bad"),
                },
                &one
            ),
            "Cannot calculate size of /a/b: bad"
        );
    }

    #[test]
    fn measured_lines() {
        let one = [OsString::from("docs")];
        let two = [OsString::from("a"), OsString::from("b")];
        let line = |kind, outcome: &Outcome, targets: &[OsString], size_format| {
            OutcomeLine {
                kind,
                outcome,
                targets,
                size_format,
            }
            .to_string()
        };

        assert_eq!(
            line(
                OperationKind::CalculateSize,
                &Outcome::Measured {
                    files: 1,
                    directories: 1,
                    bytes: 1234,
                    unreadable: 0,
                },
                &one,
                SizeFormat::Comma
            ),
            "Size of docs: 1,234 bytes (1 file, 1 directory)"
        );
        assert_eq!(
            line(
                OperationKind::CalculateSize,
                &Outcome::Measured {
                    files: 0,
                    directories: 2,
                    bytes: 2048,
                    unreadable: 3,
                },
                &two,
                SizeFormat::Human
            ),
            "Size of 2 items: 2.0K (0 files, 2 directories, 3 unreadable)"
        );
        assert_eq!(
            line(
                OperationKind::CalculateSizeOnDisk,
                &Outcome::Measured {
                    files: 2,
                    directories: 0,
                    bytes: 4096,
                    unreadable: 0,
                },
                &one,
                SizeFormat::Comma
            ),
            "Size on disk of docs: 4,096 bytes (2 files, 0 directories)"
        );
    }
}
