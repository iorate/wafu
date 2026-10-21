use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read, Write};
use std::path::Path;
use std::time::SystemTime;

use thiserror::Error;
use wafu_core::{ArchiveFormat, Entry, EntryKind, TargetKind};

use crate::time::timestamp;

mod names;
pub(crate) mod zip;

pub(crate) use names::check_component;

const CANCEL_CHECK_INTERVAL: usize = 256;

/// One entry of an archive, as read from its table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveEntry {
    /// The decoded name split at the separators, with empty parts dropped.
    pub components: Vec<String>,
    pub kind: ArchiveEntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    /// Unix permission bits, when the archive records them.
    pub mode: Option<u32>,
    pub encrypted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchiveEntryKind {
    Dir,
    File,
    /// The entry's content is the link target.
    Symlink,
}

#[derive(Debug, Error)]
pub(crate) enum ArchiveError {
    #[error("a password is required")]
    PasswordRequired,
    #[error("wrong password")]
    WrongPassword,
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// Reads one archive. Every format implements this; nothing else in the
/// crate knows what the format is.
pub(crate) trait ArchiveReader {
    fn entries(&self) -> &[ArchiveEntry];

    /// Opens an entry for reading its content. A password is only looked at
    /// for an encrypted entry.
    fn open(
        &mut self,
        index: usize,
        password: Option<&str>,
    ) -> Result<Box<dyn Read + '_>, ArchiveError>;
}

/// Writes one archive. A file is started with `start_file` and then written
/// through `Write`.
pub(crate) trait ArchiveWriter: Write {
    fn add_dir(
        &mut self,
        path: &str,
        modified: Option<SystemTime>,
        mode: Option<u32>,
    ) -> io::Result<()>;

    fn start_file(
        &mut self,
        path: &str,
        size: u64,
        modified: Option<SystemTime>,
        mode: Option<u32>,
    ) -> io::Result<()>;

    fn add_symlink(
        &mut self,
        path: &str,
        target: &str,
        modified: Option<SystemTime>,
    ) -> io::Result<()>;

    fn finish(self: Box<Self>) -> io::Result<()>;
}

pub(crate) fn open_archive(
    path: &Path,
    format: ArchiveFormat,
) -> Result<Box<dyn ArchiveReader>, ArchiveError> {
    match format {
        ArchiveFormat::Zip => Ok(Box::new(zip::ZipReader::open(path)?)),
    }
}

pub(crate) fn create_archive(
    file: File,
    format: ArchiveFormat,
    password: Option<&str>,
) -> Box<dyn ArchiveWriter> {
    match format {
        ArchiveFormat::Zip => Box::new(zip::ZipArchiveWriter::new(file, password)),
    }
}

/// The listing of one directory inside an archive. Directories that have no
/// entry of their own are made up from the paths below them. Gives up with
/// [`io::ErrorKind::Interrupted`] once `cancelled` returns `true`.
pub(crate) fn list_dir(
    entries: &[ArchiveEntry],
    inner: &[String],
    cancelled: impl Fn() -> bool,
) -> io::Result<Vec<Entry>> {
    let mut listed: Vec<Entry> = Vec::new();
    let mut index_of: HashMap<&str, usize> = HashMap::new();
    for (count, entry) in entries.iter().enumerate() {
        if count % CANCEL_CHECK_INTERVAL == 0 && cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        if entry.components.len() <= inner.len() || entry.components[..inner.len()] != *inner {
            continue;
        }
        let name = entry.components[inner.len()].as_str();
        let direct = entry.components.len() == inner.len() + 1;
        let listing = if direct {
            listed_entry(name, entry)
        } else {
            synthesized_dir(name)
        };
        // A real entry fills in what a made-up directory lacks.
        if let Some(&index) = index_of.get(name) {
            if direct {
                listed[index] = listing;
            }
        } else {
            index_of.insert(name, listed.len());
            listed.push(listing);
        }
    }
    Ok(listed)
}

/// The index of the entry named `name` directly inside the directory `inner`.
pub(crate) fn find_entry(entries: &[ArchiveEntry], inner: &[String], name: &str) -> Option<usize> {
    entries.iter().position(|entry| {
        entry.components.len() == inner.len() + 1
            && entry.components[..inner.len()] == *inner
            && entry.components[inner.len()] == name
    })
}

fn listed_entry(name: &str, entry: &ArchiveEntry) -> Entry {
    let kind = match entry.kind {
        ArchiveEntryKind::Dir => EntryKind::Dir,
        ArchiveEntryKind::File => EntryKind::File,
        // What the link points to is not known until it is extracted.
        ArchiveEntryKind::Symlink => EntryKind::Symlink {
            target: Some(TargetKind::File),
        },
    };
    Entry {
        name: OsString::from(name),
        kind,
        size: if kind == EntryKind::Dir {
            0
        } else {
            entry.size
        },
        modified: entry.modified.map(timestamp),
        hidden: is_hidden(name),
    }
}

fn synthesized_dir(name: &str) -> Entry {
    Entry {
        name: OsString::from(name),
        kind: EntryKind::Dir,
        size: 0,
        modified: None,
        hidden: is_hidden(name),
    }
}

// Archives carry no hidden attribute, so the dot rule applies everywhere.
fn is_hidden(name: &str) -> bool {
    name.starts_with('.')
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn lists_the_direct_children_of_a_directory() {
        let entries = [
            dir(&["docs"]),
            file(&["docs", "a.txt"], 3),
            file(&["docs", "sub", "b.txt"], 4),
            file(&["top.txt"], 5),
        ];

        let root = list_dir(&entries, &[], || false).unwrap();
        assert_eq!(names(&root), ["docs", "top.txt"]);
        assert_eq!(root[0].kind, EntryKind::Dir);
        assert!(root[0].modified.is_some());

        let docs = list_dir(&entries, &["docs".to_owned()], || false).unwrap();
        assert_eq!(names(&docs), ["a.txt", "sub"]);
        assert_eq!(docs[0].size, 3);
        assert_eq!(docs[1].kind, EntryKind::Dir);
        assert_eq!(docs[1].modified, None);
    }

    #[test]
    fn a_directory_entry_after_its_children_still_brings_its_time() {
        let entries = [file(&["d", "x"], 1), dir(&["d"])];

        let root = list_dir(&entries, &[], || false).unwrap();

        assert_eq!(names(&root), ["d"]);
        assert!(root[0].modified.is_some());
    }

    #[test]
    fn links_are_listed_as_links_to_files() {
        let entries = [ArchiveEntry {
            kind: ArchiveEntryKind::Symlink,
            ..file(&["l"], 2)
        }];

        let root = list_dir(&entries, &[], || false).unwrap();

        assert_eq!(
            root[0].kind,
            EntryKind::Symlink {
                target: Some(TargetKind::File)
            }
        );
        assert_eq!(root[0].size, 2);
    }

    #[test]
    fn dot_names_are_hidden() {
        let entries = [file(&[".hidden"], 0), file(&["shown"], 0)];

        let root = list_dir(&entries, &[], || false).unwrap();

        assert!(root[0].hidden);
        assert!(!root[1].hidden);
    }

    #[test]
    fn find_entry_matches_the_whole_name() {
        let entries = [file(&["d", "a.txt"], 1), file(&["a.txt"], 2), dir(&["d"])];

        assert_eq!(find_entry(&entries, &[], "a.txt"), Some(1));
        assert_eq!(find_entry(&entries, &["d".to_owned()], "a.txt"), Some(0));
        assert_eq!(find_entry(&entries, &[], "d"), Some(2));
        assert_eq!(find_entry(&entries, &[], "b.txt"), None);
    }

    #[test]
    fn a_cancelled_listing_is_interrupted() {
        let entries = [file(&["a"], 0)];

        let error = list_dir(&entries, &[], || true).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    }

    fn file(components: &[&str], size: u64) -> ArchiveEntry {
        ArchiveEntry {
            components: components.iter().map(|c| (*c).to_owned()).collect(),
            kind: ArchiveEntryKind::File,
            size,
            modified: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
            mode: None,
            encrypted: false,
        }
    }

    fn dir(components: &[&str]) -> ArchiveEntry {
        ArchiveEntry {
            kind: ArchiveEntryKind::Dir,
            ..file(components, 0)
        }
    }

    fn names(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| entry.name.to_string_lossy().into_owned())
            .collect()
    }
}
