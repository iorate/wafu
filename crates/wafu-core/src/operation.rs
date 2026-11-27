use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use crate::config::{SizeFormat, byte_count};
use crate::error::IoError;
use crate::filter::Filter;
use crate::location::{ArchiveFormat, ArchivePath, Passwords};

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
    /// Entries of one directory inside an archive. For
    /// [`OperationKind::Extract`], each names an archive to unpack.
    Entries(ArchiveEntries),
    /// The directory a [`OperationKind::Find`] searches and what it looks for.
    Find(FindQuery),
    /// What a [`OperationKind::Rename`] renames.
    Renames(Renames),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntries {
    pub archive: PathBuf,
    /// The directory the entries are in.
    pub path: ArchivePath,
    /// In display order.
    pub names: Vec<String>,
    /// The passwords the pane knows for the archive levels of `path`; the
    /// worker asks for any it needs and has not. For
    /// [`OperationKind::Extract`], the archives unpacked are one level past
    /// these.
    pub passwords: Passwords,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveOutput {
    pub format: ArchiveFormat,
    pub name: OsString,
    /// `None` writes without encryption.
    pub password: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Renames {
    pub dir: PathBuf,
    /// `(old name, new name)` of each entry of `dir` whose name changes, in
    /// display order.
    pub pairs: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindQuery {
    pub dir: PathBuf,
    /// Applied to the path of each entry below `dir`.
    pub pattern: Filter,
    /// Whether hidden entries are found and hidden directories searched.
    pub show_hidden: bool,
    /// How many matches the find collects before it stops.
    pub max_matches: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Copy,
    Move,
    Delete,
    Trash,
    /// Unpacks whole archives, files or entries of an archive, into the
    /// destination, each into a directory named after it when `into_dir`.
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
    /// Collects the entries below a directory that match a pattern, without
    /// following links.
    Find,
    /// Renames entries of one directory, in an order that frees each new name
    /// before it is taken.
    Rename,
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
    /// For [`Sources::Renames`], `index` is into `pairs`.
    ItemDone {
        index: usize,
    },
    /// A rename failed partway through renaming the items at `cycle` (indices
    /// into `pairs`), each of which takes the old name of another: the item at
    /// `left` is under the temporary name `temp`, and the others in `cycle` may
    /// or may not have been renamed. Reported just before
    /// [`Outcome::Failed`], and only once something in the cycle was renamed.
    CycleBroken {
        cycle: Vec<usize>,
        left: usize,
        temp: String,
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
    /// What the worker reads is encrypted in the archive at `level` of the
    /// sources (see [`Passwords`]), shown as `archive`; the worker waits for a
    /// password. `retry` means the one it was given was wrong.
    NeedPassword {
        archive: PathBuf,
        level: usize,
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
    /// The end of a `Find`, sorted by path. `truncated` means it stopped at
    /// the most it collects.
    Found {
        paths: Vec<PathBuf>,
        truncated: bool,
    },
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
    /// Whether the operation is a copy from the clipboard, which the line
    /// calls a paste.
    pub paste: bool,
    pub outcome: &'a Outcome,
    /// The names of what the operation was started on.
    pub targets: &'a [OsString],
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
            Self::Find => "search",
            Self::Rename => "rename",
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
            Self::Find => "Finding",
            Self::Rename => "Renaming",
        }
    }

    /// The word after the count in the progress popup: `3 done`.
    #[must_use]
    pub fn progress(self) -> &'static str {
        match self {
            Self::Find => "found",
            _ => "done",
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
            Self::Find => "Found",
            Self::Rename => "Renamed",
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
            Self::Find => "Find",
            Self::Rename => "Rename",
        }
    }
}

impl fmt::Display for OutcomeLine<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = self.kind;
        let (verb, past, noun) = if self.paste {
            ("paste", "Pasted", "Paste")
        } else {
            (kind.verb(), kind.past(), kind.noun())
        };
        match self.outcome {
            Outcome::Completed { skipped } => {
                write!(f, "{past} {}", Subject(self.targets))?;
                if kind == OperationKind::Trash {
                    f.write_str(" to the trash")?;
                }
                if *skipped > 0 {
                    write!(f, " ({skipped} skipped)")?;
                }
                Ok(())
            }
            Outcome::Cancelled => write!(f, "{noun} cancelled"),
            Outcome::Found { paths, .. } => {
                let count = paths.len();
                let unit = plural(count, "match", "matches");
                write!(f, "{noun} stopped at {count} {unit}")
            }
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
                    "{size} of {}: {}",
                    Subject(self.targets),
                    byte_count(*bytes)
                )?;
                if *bytes >= 1024 {
                    write!(f, " ({}B)", SizeFormat::Human.format(*bytes))?;
                }
                write!(
                    f,
                    " in {directories} {} and {files} {}",
                    plural(*directories, "directory", "directories"),
                    plural(*files, "file", "files"),
                )?;
                if *unreadable > 0 {
                    write!(f, " ({unreadable} unreadable)")?;
                }
                Ok(())
            }
            Outcome::Failed { path, error } => {
                write!(f, "Cannot {verb} {}: {error}", path.display())
            }
        }
    }
}

pub(crate) struct Subject<'a>(pub(crate) &'a [OsString]);

impl fmt::Display for Subject<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0 {
            [name] => write!(f, "{}", name.to_string_lossy()),
            targets => write!(f, "{} items", targets.len()),
        }
    }
}

pub(crate) fn plural(count: usize, one: &'static str, many: &'static str) -> &'static str {
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
                paste: false,
                outcome,
                targets,
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
            line(OperationKind::Find, &Outcome::Cancelled, &one),
            "Find cancelled"
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
    fn paste_lines_call_the_copy_a_paste() {
        let three = [
            OsString::from("a"),
            OsString::from("b"),
            OsString::from("c"),
        ];
        let line = |outcome: &Outcome| {
            OutcomeLine {
                kind: OperationKind::Copy,
                paste: true,
                outcome,
                targets: &three,
            }
            .to_string()
        };

        assert_eq!(
            line(&Outcome::Completed { skipped: 2 }),
            "Pasted 3 items (2 skipped)"
        );
        assert_eq!(line(&Outcome::Cancelled), "Paste cancelled");
        assert_eq!(
            line(&Outcome::Failed {
                path: PathBuf::from("/a/b"),
                error: IoError::new("permission denied"),
            }),
            "Cannot paste /a/b: permission denied"
        );
    }

    #[test]
    fn a_find_stopped_at_one_match_says_match() {
        let line = OutcomeLine {
            kind: OperationKind::Find,
            paste: false,
            outcome: &Outcome::Found {
                paths: vec![PathBuf::from("a")],
                truncated: true,
            },
            targets: &[OsString::from("docs")],
        };

        assert_eq!(line.to_string(), "Find stopped at 1 match");
    }

    #[test]
    fn measured_lines() {
        let one = [OsString::from("docs")];
        let two = [OsString::from("a"), OsString::from("b")];
        let line = |kind, outcome: &Outcome, targets: &[OsString]| {
            OutcomeLine {
                kind,
                paste: false,
                outcome,
                targets,
            }
            .to_string()
        };

        assert_eq!(
            line(
                OperationKind::CalculateSize,
                &Outcome::Measured {
                    files: 12,
                    directories: 3,
                    bytes: 48_318_382_080,
                    unreadable: 0,
                },
                &one
            ),
            "Size of docs: 48,318,382,080 bytes (45GB) in 3 directories and 12 files"
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
                &one
            ),
            "Size on disk of docs: 4,096 bytes (4.0KB) in 0 directories and 2 files"
        );
        assert_eq!(
            line(
                OperationKind::CalculateSize,
                &Outcome::Measured {
                    files: 0,
                    directories: 2,
                    bytes: 1024,
                    unreadable: 3,
                },
                &two
            ),
            "Size of 2 items: 1,024 bytes (1.0KB) in 2 directories and 0 files (3 unreadable)"
        );
        assert_eq!(
            line(
                OperationKind::CalculateSize,
                &Outcome::Measured {
                    files: 1,
                    directories: 0,
                    bytes: 1023,
                    unreadable: 0,
                },
                &one
            ),
            "Size of docs: 1,023 bytes in 0 directories and 1 file"
        );
    }
}
