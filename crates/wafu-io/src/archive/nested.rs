use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use wafu_core::{ArchivePath, Entry, OuterArchive};

use super::shared::SharedFile;
use super::{
    ArchiveEntry, ArchiveEntryKind, ArchiveError, ArchiveReader, NOT_FOUND, find_entry,
    listed_entry, open_archive, open_file,
};
use crate::runtime::lock;

const CHUNK_SIZE: usize = 64 * 1024;
const NOT_REGULAR: &str = "not a regular file";

// What the caches of an archive check it by: one whose length or
// modification time differs is read again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
}

impl Stamp {
    pub(crate) fn of(path: &Path) -> io::Result<Self> {
        let meta = fs::metadata(path)?;
        Ok(Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

pub(crate) struct OpenedArchive {
    pub reader: Box<dyn ArchiveReader>,
    // A nested archive as an entry of the archive holding it; `None` for an
    // archive that is a file on disk.
    pub own: Option<Entry>,
}

// The archives inside archives that the reader lanes have copied out, so
// that listing one, previewing an entry of it, and copying out of it copy it
// out once. A lane keeps the levels of the archive it read last until it
// reads something else; a file no lane keeps is let go, and goes away once
// the readers that still have it are done.
#[derive(Default)]
pub(crate) struct NestedArchives(Mutex<Store>);

// A reader lane's way into `NestedArchives`: what it opens is kept under the
// lane's name, which outlives the lane's threads, until it reads something
// else. A reader abandoned on a cancel leaves the hold alone, so that it does
// not overwrite what the reader that took over holds.
#[derive(Clone, Copy)]
pub(crate) struct LaneArchives<'a> {
    name: &'static str,
    nested: &'a NestedArchives,
    abandoned: &'a AtomicBool,
}

impl NestedArchives {
    pub(crate) fn lane<'a>(
        &'a self,
        name: &'static str,
        abandoned: &'a AtomicBool,
    ) -> LaneArchives<'a> {
        LaneArchives {
            name,
            nested: self,
            abandoned,
        }
    }

    // For an operation: copies already made are used, and those it makes
    // last as long as its reader.
    pub(crate) fn open(
        &self,
        base: &Path,
        stamp: Stamp,
        archive: &ArchivePath,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<OpenedArchive, ArchiveError> {
        self.open_with(base, stamp, archive, false, cancelled)
    }

    // A nested archive is copied out level by level from the deepest level
    // already copied out. With `share`, a copy is kept for the others while
    // some lane holds it.
    fn open_with(
        &self,
        base: &Path,
        stamp: Stamp,
        archive: &ArchivePath,
        share: bool,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<OpenedArchive, ArchiveError> {
        let Some(last) = archive.outer.last() else {
            return Ok(OpenedArchive {
                reader: open_archive(base, archive.format)?,
                own: None,
            });
        };
        let levels = levels(base, stamp, &archive.outer);
        let found = {
            let store = lock(&self.0);
            levels.iter().enumerate().rev().find_map(|(index, level)| {
                let copied = store.files.get(level)?;
                Some((index + 1, copied.file.rewound(), copied.entry.clone()))
            })
        };
        let (start, mut file, mut entry) = match found {
            Some((start, file, entry)) => (start, file, Some(entry)),
            None => (0, SharedFile::new(File::open(base)?), None),
        };
        for (outer, level) in archive.outer.iter().zip(&levels).skip(start) {
            // The store is not locked while copying, so that a large copy
            // does not hold up the other lanes.
            let copied = copy_out(file, outer, cancelled)?;
            let (kept, kept_entry) = if share {
                lock(&self.0).keep(level, copied)
            } else {
                (copied.file, copied.entry)
            };
            file = kept;
            entry = Some(kept_entry);
        }
        let name = last.entry.last().map_or("", String::as_str);
        Ok(OpenedArchive {
            reader: open_file(file, archive.format)?,
            own: entry.map(|entry| listed_entry(name, &entry)),
        })
    }
}

impl LaneArchives<'_> {
    // In place of what the lane held before: the levels of the archive
    // `outer` leads to, none when it is empty.
    pub(crate) fn hold(self, base: &Path, stamp: Stamp, outer: &[OuterArchive]) {
        if self.abandoned.load(Ordering::SeqCst) {
            return;
        }
        let mut store = lock(&self.nested.0);
        store.held.insert(self.name, levels(base, stamp, outer));
        store.prune();
    }

    pub(crate) fn release(self) {
        if self.abandoned.load(Ordering::SeqCst) {
            return;
        }
        let mut store = lock(&self.nested.0);
        store.held.remove(self.name);
        store.prune();
    }

    pub(crate) fn open(
        self,
        base: &Path,
        stamp: Stamp,
        archive: &ArchivePath,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<OpenedArchive, ArchiveError> {
        self.nested.open_with(base, stamp, archive, true, cancelled)
    }
}

#[derive(Default)]
struct Store {
    files: HashMap<Level, Copied>,
    held: HashMap<&'static str, Vec<Level>>,
}

// One archive of a chain: its base as stamped, and the entries that lead to
// it from there.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Level {
    base: PathBuf,
    stamp: Stamp,
    chain: Vec<OuterArchive>,
}

struct Copied {
    file: SharedFile,
    entry: ArchiveEntry,
}

impl Store {
    // A copy no lane holds is handed back without being kept. Another lane
    // may have copied out the same level meanwhile; the first copy stays and
    // this one is dropped.
    fn keep(&mut self, level: &Level, copied: Copied) -> (SharedFile, ArchiveEntry) {
        if !self.held.values().flatten().any(|held| held == level) {
            return (copied.file, copied.entry);
        }
        let kept = self.files.entry(level.clone()).or_insert(copied);
        (kept.file.rewound(), kept.entry.clone())
    }

    fn prune(&mut self) {
        let Self { files, held } = self;
        files.retain(|level, _| held.values().flatten().any(|held| held == level));
    }
}

fn levels(base: &Path, stamp: Stamp, outer: &[OuterArchive]) -> Vec<Level> {
    (1..=outer.len())
        .map(|len| Level {
            base: base.to_path_buf(),
            stamp,
            chain: outer[..len].to_vec(),
        })
        .collect()
}

// Outer archives are read without a password: an encrypted one fails here
// with `ArchiveError::PasswordRequired`.
fn copy_out(
    file: SharedFile,
    outer: &OuterArchive,
    cancelled: &dyn Fn() -> bool,
) -> Result<Copied, ArchiveError> {
    let mut reader = open_file(file, outer.format)?;
    let not_found = || ArchiveError::Invalid(NOT_FOUND.to_owned());
    let (name, dir) = outer.entry.split_last().ok_or_else(not_found)?;
    let index = find_entry(reader.entries(), dir, name).ok_or_else(not_found)?;
    let entry = reader.entries()[index].clone();
    if entry.kind != ArchiveEntryKind::File {
        return Err(ArchiveError::Invalid(NOT_REGULAR.to_owned()));
    }
    let mut source = reader.open(index, None)?;
    // A file with no name, which the OS removes once the last handle to it
    // is closed, even after a crash: nothing to clean up, nothing left
    // behind, and wafu does not hold the archive in memory.
    let mut copy = tempfile::tempfile()?;
    let mut buffer = vec![0; CHUNK_SIZE];
    loop {
        if cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted).into());
        }
        let read = match source.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        };
        copy.write_all(&buffer[..read])?;
    }
    Ok(Copied {
        file: SharedFile::new(copy),
        entry,
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use wafu_core::{ArchiveFormat, EntryKind};

    use super::*;
    use crate::archive::create_archive;
    use crate::archive::tests::{archive_bytes, write_archive};

    static LIVE: AtomicBool = AtomicBool::new(false);

    #[test]
    fn a_nested_archive_is_read_through_each_kind_of_outer_archive_to_any_depth() {
        use ArchiveFormat::{TarGz, Zip};
        for formats in [
            &[Zip, Zip][..],
            &[Zip, TarGz][..],
            &[TarGz, Zip][..],
            &[Zip, TarGz, Zip][..],
        ] {
            let dir = TempDir::new().unwrap();
            let (base, archive) = nest(dir.path(), formats, b"inner");

            let mut opened = NestedArchives::default()
                .open(&base, stamp(&base), &archive, &|| false)
                .unwrap();

            assert_eq!(content(&mut opened, "y/f.txt"), b"inner", "{formats:?}");
            let own = opened.own.unwrap();
            assert_eq!(own.name, "n", "{formats:?}");
            assert_eq!(own.kind, EntryKind::File { executable: false });
        }
    }

    #[test]
    fn the_formats_of_a_nested_archive_and_its_outer_archive_are_read_off_the_content() {
        let dir = TempDir::new().unwrap();
        let (base, mut archive) = nest(
            dir.path(),
            &[ArchiveFormat::Zip, ArchiveFormat::TarGz, ArchiveFormat::Zip],
            b"inner",
        );
        archive.outer[1].format = None;
        archive.format = None;

        let mut opened = NestedArchives::default()
            .open(&base, stamp(&base), &archive, &|| false)
            .unwrap();

        assert_eq!(content(&mut opened, "y/f.txt"), b"inner");
    }

    #[test]
    fn an_outer_entry_that_is_missing_or_not_a_file_fails() {
        let dir = TempDir::new().unwrap();
        let base = dir.path().join("a.zip");
        let mut writer =
            create_archive(File::create(&base).unwrap(), ArchiveFormat::Zip, None).unwrap();
        writer.add_dir("x", None, None).unwrap();
        writer.add_symlink("l", "x", None).unwrap();
        writer.finish().unwrap();

        for (entry, message) in [
            (&["x", "g.zip"][..], NOT_FOUND),
            (&["x"][..], NOT_REGULAR),
            (&["l"][..], NOT_REGULAR),
        ] {
            let archive = ArchivePath {
                outer: vec![OuterArchive {
                    format: Some(ArchiveFormat::Zip),
                    entry: strings(entry),
                }],
                format: None,
                inner: Vec::new(),
            };

            let error = NestedArchives::default()
                .open(&base, stamp(&base), &archive, &|| false)
                .err()
                .unwrap();

            assert_eq!(error.to_string(), message, "{entry:?}");
        }
    }

    #[test]
    fn a_cancelled_copy_is_interrupted() {
        let dir = TempDir::new().unwrap();
        let (base, archive) = nest(dir.path(), &[ArchiveFormat::Zip, ArchiveFormat::Zip], b"a");

        let error = NestedArchives::default()
            .open(&base, stamp(&base), &archive, &|| true)
            .err()
            .unwrap();

        assert!(
            matches!(&error, ArchiveError::Io(error) if error.kind() == io::ErrorKind::Interrupted),
            "{error:?}"
        );
    }

    #[test]
    fn a_copy_one_lane_holds_is_used_by_other_lanes_and_operations() {
        let dir = TempDir::new().unwrap();
        let (base, archive) = nest(dir.path(), &[ArchiveFormat::Zip, ArchiveFormat::Zip], b"a");
        let nested = NestedArchives::default();
        let stamp = stamp(&base);
        let left = nested.lane("left", &LIVE);
        left.hold(&base, stamp, &archive.outer);
        left.open(&base, stamp, &archive, &|| false).unwrap();
        spoil_keeping_the_stamp(&base);

        let right = nested.lane("right", &LIVE);
        right.hold(&base, stamp, &archive.outer);
        let mut opened = right.open(&base, stamp, &archive, &|| false).unwrap();
        assert_eq!(content(&mut opened, "y/f.txt"), b"a");

        let mut opened = nested.open(&base, stamp, &archive, &|| false).unwrap();
        assert_eq!(content(&mut opened, "y/f.txt"), b"a");
    }

    #[test]
    fn a_copy_is_let_go_once_no_lane_holds_it() {
        let dir = TempDir::new().unwrap();
        let (base, archive) = nest(dir.path(), &[ArchiveFormat::Zip, ArchiveFormat::Zip], b"a");
        let nested = NestedArchives::default();
        let stamp = stamp(&base);
        let (left, right) = (nested.lane("left", &LIVE), nested.lane("right", &LIVE));
        left.hold(&base, stamp, &archive.outer);
        right.hold(&base, stamp, &archive.outer);
        left.open(&base, stamp, &archive, &|| false).unwrap();

        left.release();
        assert_eq!(copies(&nested), 1);

        right.hold(&base, stamp, &[]);
        assert_eq!(copies(&nested), 0);
    }

    #[test]
    fn an_abandoned_reader_leaves_the_hold_of_its_lane_alone() {
        let dir = TempDir::new().unwrap();
        let (base, archive) = nest(dir.path(), &[ArchiveFormat::Zip, ArchiveFormat::Zip], b"a");
        let nested = NestedArchives::default();
        let stamp = stamp(&base);
        let left = nested.lane("left", &LIVE);
        left.hold(&base, stamp, &archive.outer);
        left.open(&base, stamp, &archive, &|| false).unwrap();

        let abandoned = AtomicBool::new(true);
        nested.lane("left", &abandoned).release();

        assert_eq!(copies(&nested), 1);
    }

    #[test]
    fn copies_made_by_an_operation_or_for_no_holder_are_not_kept() {
        let dir = TempDir::new().unwrap();
        let (base, archive) = nest(dir.path(), &[ArchiveFormat::Zip, ArchiveFormat::Zip], b"a");
        let nested = NestedArchives::default();
        let stamp = stamp(&base);

        nested.open(&base, stamp, &archive, &|| false).unwrap();
        assert_eq!(copies(&nested), 0);

        nested
            .lane("left", &LIVE)
            .open(&base, stamp, &archive, &|| false)
            .unwrap();
        assert_eq!(copies(&nested), 0);
    }

    #[test]
    fn a_changed_base_is_copied_out_again() {
        let dir = TempDir::new().unwrap();
        let (base, archive) = nest(dir.path(), &[ArchiveFormat::Zip, ArchiveFormat::Zip], b"a");
        let nested = NestedArchives::default();
        let left = nested.lane("left", &LIVE);
        let old = stamp(&base);
        left.hold(&base, old, &archive.outer);
        left.open(&base, old, &archive, &|| false).unwrap();

        nest(
            dir.path(),
            &[ArchiveFormat::Zip, ArchiveFormat::Zip],
            b"longer",
        );
        let new = stamp(&base);
        let mut opened = nested.open(&base, new, &archive, &|| false).unwrap();

        assert_eq!(content(&mut opened, "y/f.txt"), b"longer");
    }

    // `a` in `dir`, holding `x/n` in the first format, which holds `n` in
    // the next, and so on; the last holds `y/f.txt` with `data`.
    fn nest(dir: &Path, formats: &[ArchiveFormat], data: &[u8]) -> (PathBuf, ArchivePath) {
        let (&innermost, outers) = formats.split_last().unwrap();
        let mut bytes = archive_bytes(innermost, &[("y/f.txt", data)]);
        let mut outer = Vec::new();
        for (depth, &format) in outers.iter().enumerate().rev() {
            let name = if depth == 0 { "x/n" } else { "n" };
            outer.insert(
                0,
                OuterArchive {
                    format: Some(format),
                    entry: name.split('/').map(ToOwned::to_owned).collect(),
                },
            );
            if depth == 0 {
                let base = dir.join("a");
                write_archive(&base, format, &[(name, &bytes)]);
                let archive = ArchivePath {
                    outer,
                    format: Some(innermost),
                    inner: Vec::new(),
                };
                return (base, archive);
            }
            bytes = archive_bytes(format, &[(name, &bytes)]);
        }
        unreachable!("a nested archive has an outer archive");
    }

    fn stamp(path: &Path) -> Stamp {
        Stamp::of(path).unwrap()
    }

    // Unreadable as an archive, though its length and time are as before.
    fn spoil_keeping_the_stamp(path: &Path) {
        let modified = fs::metadata(path).unwrap().modified().unwrap();
        let len = fs::metadata(path).unwrap().len();
        let zeros = vec![0; usize::try_from(len).unwrap()];
        fs::write(path, zeros).unwrap();
        File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(modified)
            .unwrap();
    }

    fn content(opened: &mut OpenedArchive, path: &str) -> Vec<u8> {
        let components = strings(&path.split('/').collect::<Vec<_>>());
        let index = opened
            .reader
            .entries()
            .iter()
            .position(|entry| entry.components == components)
            .unwrap();
        let mut data = Vec::new();
        opened
            .reader
            .open(index, None)
            .unwrap()
            .read_to_end(&mut data)
            .unwrap();
        data
    }

    fn copies(nested: &NestedArchives) -> usize {
        lock(&nested.0).files.len()
    }

    fn strings(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }
}
