use std::collections::HashMap;
use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::path::Path;
use std::time::SystemTime;

use thiserror::Error;
use wafu_core::{ArchiveFormat, Entry, EntryKind, Permissions, TargetKind};

use crate::time::timestamp;

mod names;
pub(crate) mod tar;
pub(crate) mod zip;

pub(crate) use names::check_component;

use self::tar::Compression;

const CANCEL_CHECK_INTERVAL: usize = 256;
// Enough for the `ustar` magic at byte 257.
const SNIFF_LEN: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ArchiveEntry {
    pub components: Vec<String>,
    pub kind: ArchiveEntryKind,
    pub size: u64,
    pub modified: Option<SystemTime>,
    // Unix permission bits, when the archive records them.
    pub mode: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ArchiveEntryKind {
    Dir,
    File,
    // The entry's content is the link target.
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

// Reads one archive. Every format implements this; nothing else in the
// crate knows what the format is.
pub(crate) trait ArchiveReader {
    fn entries(&self) -> &[ArchiveEntry];

    // Opens an entry for reading its content. A password is only looked at
    // for an encrypted entry.
    fn open(
        &mut self,
        index: usize,
        password: Option<&str>,
    ) -> Result<Box<dyn Read + '_>, ArchiveError>;

    // The earlier entry whose content a hard link at `index` shares.
    fn hard_link_target(&self, _index: usize) -> Option<usize> {
        None
    }
}

// Writes one archive. A file's content is pulled from `data`, which is
// what both the zip and the tar crate can take.
pub(crate) trait ArchiveWriter {
    fn add_dir(
        &mut self,
        path: &str,
        modified: Option<SystemTime>,
        mode: Option<u32>,
    ) -> io::Result<()>;

    fn add_file(
        &mut self,
        path: &str,
        size: u64,
        modified: Option<SystemTime>,
        mode: Option<u32>,
        data: &mut dyn Read,
    ) -> io::Result<()>;

    fn add_symlink(
        &mut self,
        path: &str,
        target: &str,
        modified: Option<SystemTime>,
    ) -> io::Result<()>;

    fn finish(self: Box<Self>) -> io::Result<()>;
}

// Opens an archive whose format is known, or read off its content when
// `format` is `None`.
pub(crate) fn open_archive(
    path: &Path,
    format: Option<ArchiveFormat>,
) -> Result<Box<dyn ArchiveReader>, ArchiveError> {
    let format = match format {
        Some(format) => format,
        None => detect_format(path)?,
    };
    match compression(format) {
        None => Ok(Box::new(zip::ZipReader::open(path)?)),
        Some(compression) => Ok(Box::new(tar::TarReader::open(path, compression)?)),
    }
}

pub(crate) fn create_archive(
    file: File,
    format: ArchiveFormat,
    password: Option<&str>,
) -> io::Result<Box<dyn ArchiveWriter>> {
    match compression(format) {
        None => Ok(Box::new(zip::ZipArchiveWriter::new(file, password))),
        Some(compression) => Ok(Box::new(tar::TarWriter::new(file, compression)?)),
    }
}

// The compression around a tar; `None` for zip.
fn compression(format: ArchiveFormat) -> Option<Compression> {
    match format {
        ArchiveFormat::Zip => None,
        ArchiveFormat::Tar => Some(Compression::None),
        ArchiveFormat::TarGz => Some(Compression::Gzip),
        ArchiveFormat::TarBz2 => Some(Compression::Bzip2),
        ArchiveFormat::TarXz => Some(Compression::Xz),
    }
}

fn detect_format(path: &Path) -> io::Result<ArchiveFormat> {
    let mut head = [0; SNIFF_LEN];
    let mut file = File::open(path)?;
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(sniff(&head[..filled]))
}

// Zip is the fallback, as it was when every archive was one. A compressed
// stream is taken to hold a tar; what else it could hold is not read.
fn sniff(head: &[u8]) -> ArchiveFormat {
    if head.starts_with(&[0x1f, 0x8b]) {
        ArchiveFormat::TarGz
    } else if head.starts_with(b"BZh") {
        ArchiveFormat::TarBz2
    } else if head.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) {
        ArchiveFormat::TarXz
    } else if head.get(257..262) == Some(b"ustar") {
        ArchiveFormat::Tar
    } else {
        ArchiveFormat::Zip
    }
}

// The listing of one directory inside an archive. Directories that have no
// entry of their own are made up from the paths below them. Gives up with
// `io::ErrorKind::Interrupted` once `cancelled` returns `true`.
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

// The directory `inner` itself: its own entry when the archive has one, else
// a directory made up as `list_dir` does. `None` at the root, which has no
// entry of its own.
pub(crate) fn dir_entry(entries: &[ArchiveEntry], inner: &[String]) -> Option<Entry> {
    let name = inner.last()?;
    let own = entries
        .iter()
        .find(|entry| entry.kind == ArchiveEntryKind::Dir && entry.components == inner);
    Some(match own {
        Some(entry) => listed_entry(name, entry),
        None => synthesized_dir(name),
    })
}

// The index of the last entry named `name` directly inside the directory
// `inner`.
pub(crate) fn find_entry(entries: &[ArchiveEntry], inner: &[String], name: &str) -> Option<usize> {
    entries.iter().rposition(|entry| {
        entry.components.len() == inner.len() + 1
            && entry.components[..inner.len()] == *inner
            && entry.components[inner.len()] == name
    })
}

fn listed_entry(name: &str, entry: &ArchiveEntry) -> Entry {
    let kind = match entry.kind {
        ArchiveEntryKind::Dir => EntryKind::Dir,
        ArchiveEntryKind::File => EntryKind::File {
            executable: entry.mode.is_some_and(|mode| mode & 0o111 != 0),
        },
        // What the link points to is not known until it is extracted.
        ArchiveEntryKind::Symlink => EntryKind::Symlink {
            target: Some(TargetKind::File { executable: false }),
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
        modified: entry.modified.and_then(timestamp),
        hidden: is_hidden(name),
        permissions: entry.mode.map(|mode| Permissions::Mode(mode & 0o7777)),
        link_target: None,
    }
}

fn synthesized_dir(name: &str) -> Entry {
    Entry {
        name: OsString::from(name),
        kind: EntryKind::Dir,
        size: 0,
        modified: None,
        hidden: is_hidden(name),
        permissions: None,
        link_target: None,
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
                target: Some(TargetKind::File { executable: false })
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
    fn dir_entry_comes_from_the_archive_or_is_made_up() {
        let entries = [dir(&["d"]), file(&["d", "x"], 1), file(&["e", "y"], 1)];

        assert_eq!(dir_entry(&entries, &[]), None);
        let d = dir_entry(&entries, &["d".to_owned()]).unwrap();
        assert_eq!(d.name, "d");
        assert_eq!(d.kind, EntryKind::Dir);
        assert!(d.modified.is_some());
        let e = dir_entry(&entries, &["e".to_owned()]).unwrap();
        assert_eq!(e.name, "e");
        assert_eq!(e.kind, EntryKind::Dir);
        assert_eq!(e.modified, None);
        assert_eq!(e.permissions, None);
    }

    #[test]
    fn find_entry_matches_the_whole_name() {
        let entries = [file(&["d", "a.txt"], 1), file(&["a.txt"], 2), dir(&["d"])];

        assert_eq!(find_entry(&entries, &[], "a.txt"), Some(1));
        assert_eq!(find_entry(&entries, &["d".to_owned()], "a.txt"), Some(0));
        assert_eq!(find_entry(&entries, &[], "d"), Some(2));
        assert_eq!(find_entry(&entries, &[], "b.txt"), None);

        let duplicated = [
            file(&["a.txt"], 1),
            file(&["b.txt"], 2),
            file(&["a.txt"], 3),
        ];

        assert_eq!(find_entry(&duplicated, &[], "a.txt"), Some(2));
    }

    #[test]
    fn the_format_is_read_off_the_first_bytes() {
        assert_eq!(sniff(&[0x1f, 0x8b, 0x08]), ArchiveFormat::TarGz);
        assert_eq!(sniff(b"BZh91AY"), ArchiveFormat::TarBz2);
        assert_eq!(sniff(b"\xfd7zXZ\x00"), ArchiveFormat::TarXz);
        let mut tar = [0; 512];
        tar[257..262].copy_from_slice(b"ustar");
        assert_eq!(sniff(&tar), ArchiveFormat::Tar);
        assert_eq!(sniff(b"PK\x03\x04"), ArchiveFormat::Zip);
        assert_eq!(sniff(b""), ArchiveFormat::Zip);
        assert_eq!(sniff(b"plain text"), ArchiveFormat::Zip);
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
