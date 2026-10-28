use std::ffi::OsString;
use std::fs::{self, File, FileTimes, Metadata};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use wafu_core::{IoError, OperationEvent, OperationKind, OperationSpec, Outcome, Reply, Sources};

use crate::trash::trash;

mod archives;

const CHUNK_SIZE: usize = 64 * 1024;
const NOT_REGULAR: &str = "not a regular file";
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

/// Carries out an operation, reporting through `report` and taking answers
/// to conflicts, and cancellations, from `replies`. Returns once the
/// `Finished` event has been reported. `owner_window` is the raw handle of
/// the window that owns the dialogs the OS may show (on Windows, the warning
/// that an item cannot go to the trash).
///
/// A cancellation or an error stops the operation where it is; a file being
/// written at that moment is removed.
pub fn run_operation(
    spec: &OperationSpec,
    owner_window: Option<isize>,
    replies: &Receiver<Reply>,
    report: &dyn Fn(OperationEvent),
) {
    let mut worker = Worker {
        owner_window,
        replies,
        report,
        policy: None,
        done: 0,
        skipped: 0,
        last_progress: None,
    };
    let outcome = worker.run(spec);
    report(OperationEvent::Finished(outcome));
}

struct Worker<'a> {
    owner_window: Option<isize>,
    replies: &'a Receiver<Reply>,
    report: &'a dyn Fn(OperationEvent),
    // "Apply to all" from an earlier conflict.
    policy: Option<Policy>,
    done: usize,
    skipped: usize,
    last_progress: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Policy {
    Overwrite,
    Skip,
}

// Why an operation stops early.
enum Stop {
    Cancelled,
    Failed { path: PathBuf, error: io::Error },
}

// Whether an item is copied as is, or copied because it cannot be renamed
// across devices and its source is removed afterwards.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Transfer {
    Copy,
    MoveByCopy,
}

enum Resolution {
    Overwrite,
    Skip,
    Rename(OsString),
}

// `Ok(true)` means the item was handled with nothing skipped under it.
type Step = Result<bool, Stop>;

impl Worker<'_> {
    fn run(&mut self, spec: &OperationSpec) -> Outcome {
        match self.run_all(spec) {
            Ok(()) => Outcome::Completed {
                done: self.done,
                skipped: self.skipped,
            },
            Err(Stop::Cancelled) => Outcome::Cancelled { done: self.done },
            Err(Stop::Failed { path, error }) => Outcome::Failed {
                path,
                error: IoError::new(error.to_string()),
                done: self.done,
            },
        }
    }

    fn run_all(&mut self, spec: &OperationSpec) -> Result<(), Stop> {
        match (&spec.sources, spec.kind) {
            (Sources::Paths(paths), OperationKind::Extract) => self.extract_archives(paths, spec),
            (Sources::Paths(paths), OperationKind::Compress) => self.compress(paths, spec),
            (Sources::Paths(paths), kind) => self.run_paths(paths, kind, spec),
            (Sources::Entries(entries), OperationKind::ExtractToTemp) => {
                self.extract_to_temp(entries)
            }
            (Sources::Entries(entries), _) => {
                let dest = spec
                    .dest
                    .as_deref()
                    .ok_or_else(|| failure(&entries.archive, "no destination"))?;
                self.extract_entries(entries, dest)
            }
        }
    }

    fn run_paths(
        &mut self,
        paths: &[PathBuf],
        kind: OperationKind,
        spec: &OperationSpec,
    ) -> Result<(), Stop> {
        for (index, source) in paths.iter().enumerate() {
            let complete = match kind {
                OperationKind::Copy => {
                    let dest = Self::destination(spec, source)?;
                    self.transfer_item(source, &dest, Transfer::Copy)?
                }
                OperationKind::Move => {
                    let dest = Self::destination(spec, source)?;
                    self.move_item(source, &dest)?
                }
                OperationKind::Delete => self.delete_item(source)?,
                OperationKind::Trash => self.trash_item(source)?,
                OperationKind::Extract | OperationKind::Compress | OperationKind::ExtractToTemp => {
                    return Err(failure(source, "unsupported operation"));
                }
            };
            if complete {
                (self.report)(OperationEvent::ItemDone { index });
            }
        }
        Ok(())
    }

    fn destination(spec: &OperationSpec, source: &Path) -> Result<PathBuf, Stop> {
        let dest = spec
            .dest
            .as_deref()
            .ok_or_else(|| failure(source, "no destination"))?;
        let name = source
            .file_name()
            .ok_or_else(|| failure(source, "the path has no name"))?;
        Ok(dest.join(name))
    }

    fn transfer_item(&mut self, src: &Path, dst: &Path, transfer: Transfer) -> Step {
        self.begin(src)?;
        let meta = fs::symlink_metadata(src).map_err(fail(src))?;
        let file_type = meta.file_type();
        if file_type.is_symlink() {
            self.transfer_link(src, dst, transfer)
        } else if file_type.is_dir() {
            self.transfer_dir(src, dst, &meta, transfer)
        } else if file_type.is_file() {
            self.transfer_file(src, dst, &meta, transfer)
        } else {
            Err(failure(src, NOT_REGULAR))
        }
    }

    fn transfer_dir(
        &mut self,
        src: &Path,
        dst: &Path,
        meta: &Metadata,
        transfer: Transfer,
    ) -> Step {
        if dst.starts_with(src) {
            return Err(failure(src, "the destination is inside the source"));
        }
        match fs::symlink_metadata(dst) {
            Ok(existing) if existing.is_dir() => {}
            Ok(_) => {
                return Err(failure(
                    src,
                    "the destination exists and is not a directory",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(dst).map_err(fail(src))?;
            }
            Err(error) => {
                return Err(Stop::Failed {
                    path: src.to_path_buf(),
                    error,
                });
            }
        }
        let mut complete = true;
        for child in children(src)? {
            let name = child.file_name().unwrap_or_default();
            complete &= self.transfer_item(&child, &dst.join(name), transfer)?;
        }
        // Writing the children moved the directory's time, so it is set last.
        copy_metadata(dst, meta);
        if transfer == Transfer::MoveByCopy && complete {
            remove_dir(src).map_err(fail(src))?;
        }
        self.done += 1;
        Ok(complete)
    }

    fn transfer_file(
        &mut self,
        src: &Path,
        dst: &Path,
        meta: &Metadata,
        transfer: Transfer,
    ) -> Step {
        let Some(dst) = self.clear_destination(src, dst)? else {
            return Ok(false);
        };
        self.write_file(src, &dst)?;
        copy_metadata(&dst, meta);
        if transfer == Transfer::MoveByCopy {
            remove_file(src).map_err(fail(src))?;
        }
        self.done += 1;
        Ok(true)
    }

    fn transfer_link(&mut self, src: &Path, dst: &Path, transfer: Transfer) -> Step {
        let target = fs::read_link(src).map_err(fail(src))?;
        let Some(dst) = self.clear_destination(src, dst)? else {
            return Ok(false);
        };
        make_symlink(&target, src, &dst).map_err(fail(src))?;
        if transfer == Transfer::MoveByCopy {
            remove_file(src).map_err(fail(src))?;
        }
        self.done += 1;
        Ok(true)
    }

    // Makes room for a file or link at `dst`, asking about a conflict. Returns
    // the path to write, which may have been renamed, or `None` to skip.
    fn clear_destination(&mut self, src: &Path, dst: &Path) -> Result<Option<PathBuf>, Stop> {
        let mut dst = dst.to_path_buf();
        loop {
            let existing = match fs::symlink_metadata(&dst) {
                Ok(existing) => existing,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Some(dst)),
                Err(error) => {
                    return Err(Stop::Failed {
                        path: src.to_path_buf(),
                        error,
                    });
                }
            };
            if existing.is_dir() {
                return Err(failure(src, "the destination exists and is a directory"));
            }
            match self.resolve_conflict(src, &dst)? {
                Resolution::Overwrite => {
                    if same_file::is_same_file(src, &dst).unwrap_or(false) {
                        return Err(failure(src, "the destination is the same file"));
                    }
                    // Writing through a link would change its target instead.
                    remove_file(&dst).map_err(fail(src))?;
                    return Ok(Some(dst));
                }
                Resolution::Skip => {
                    self.skipped += 1;
                    return Ok(None);
                }
                Resolution::Rename(name) => {
                    let parent = dst.parent().map(Path::to_path_buf).unwrap_or_default();
                    dst = parent.join(name);
                }
            }
        }
    }

    fn resolve_conflict(&mut self, src: &Path, dst: &Path) -> Result<Resolution, Stop> {
        match self.policy {
            Some(Policy::Overwrite) => return Ok(Resolution::Overwrite),
            Some(Policy::Skip) => return Ok(Resolution::Skip),
            None => {}
        }
        (self.report)(OperationEvent::Conflict {
            source: src.to_path_buf(),
            dest: dst.to_path_buf(),
        });
        loop {
            match self.replies.recv() {
                Ok(Reply::Overwrite { all }) => {
                    if all {
                        self.policy = Some(Policy::Overwrite);
                    }
                    return Ok(Resolution::Overwrite);
                }
                Ok(Reply::Skip { all }) => {
                    if all {
                        self.policy = Some(Policy::Skip);
                    }
                    return Ok(Resolution::Skip);
                }
                Ok(Reply::Rename(name)) => return Ok(Resolution::Rename(name)),
                Ok(Reply::Cancel) | Err(_) => return Err(Stop::Cancelled),
                // Not an answer to this question.
                Ok(Reply::Password(_)) => {}
            }
        }
    }

    // Copies the contents in chunks, checking for a cancellation between
    // them. Whatever was written is removed when the copy does not finish.
    fn write_file(&mut self, src: &Path, dst: &Path) -> Result<(), Stop> {
        let result = self.write_chunks(src, dst);
        if result.is_err() {
            let _ = fs::remove_file(dst);
        }
        result
    }

    fn write_chunks(&mut self, src: &Path, dst: &Path) -> Result<(), Stop> {
        let mut reader = File::open(src).map_err(fail(src))?;
        let mut writer = File::create(dst).map_err(fail(src))?;
        let mut buffer = vec![0; CHUNK_SIZE];
        loop {
            self.check_cancel()?;
            let read = reader.read(&mut buffer).map_err(fail(src))?;
            if read == 0 {
                break;
            }
            writer.write_all(&buffer[..read]).map_err(fail(src))?;
        }
        writer.flush().map_err(fail(src))?;
        Ok(())
    }

    fn move_item(&mut self, src: &Path, dst: &Path) -> Step {
        self.begin(src)?;
        let meta = fs::symlink_metadata(src).map_err(fail(src))?;
        if meta.is_dir() && dst.starts_with(src) {
            return Err(failure(src, "the destination is inside the source"));
        }
        let existing = match fs::symlink_metadata(dst) {
            Ok(existing) => existing,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return self.move_to_free_path(src, dst);
            }
            Err(error) => {
                return Err(Stop::Failed {
                    path: src.to_path_buf(),
                    error,
                });
            }
        };
        if meta.is_dir() && existing.is_dir() {
            let mut complete = true;
            for child in children(src)? {
                let name = child.file_name().unwrap_or_default();
                complete &= self.move_item(&child, &dst.join(name))?;
            }
            if complete {
                remove_dir(src).map_err(fail(src))?;
            }
            self.done += 1;
            return Ok(complete);
        }
        if meta.is_dir() || existing.is_dir() {
            return Err(failure(
                src,
                "the destination exists and is of another kind",
            ));
        }
        match self.resolve_conflict(src, dst)? {
            Resolution::Overwrite => {
                remove_file(dst).map_err(fail(src))?;
                self.move_to_free_path(src, dst)
            }
            Resolution::Skip => {
                self.skipped += 1;
                Ok(false)
            }
            Resolution::Rename(name) => {
                let parent = dst.parent().map(Path::to_path_buf).unwrap_or_default();
                self.move_item(src, &parent.join(name))
            }
        }
    }

    // A rename when the file system allows it; copying and removing when it
    // crosses devices.
    fn move_to_free_path(&mut self, src: &Path, dst: &Path) -> Step {
        match fs::rename(src, dst) {
            Ok(()) => {
                self.done += 1;
                Ok(true)
            }
            Err(error) if error.kind() == io::ErrorKind::CrossesDevices => {
                self.transfer_item(src, dst, Transfer::MoveByCopy)
            }
            Err(error) => Err(Stop::Failed {
                path: src.to_path_buf(),
                error,
            }),
        }
    }

    fn delete_item(&mut self, path: &Path) -> Step {
        self.begin(path)?;
        let meta = fs::symlink_metadata(path).map_err(fail(path))?;
        if meta.is_symlink() {
            remove_file(path).map_err(fail(path))?;
        } else if meta.is_dir() {
            for child in children(path)? {
                self.delete_item(&child)?;
            }
            remove_dir(path).map_err(fail(path))?;
        } else {
            remove_file(path).map_err(fail(path))?;
        }
        self.done += 1;
        Ok(true)
    }

    fn trash_item(&mut self, path: &Path) -> Step {
        self.begin(path)?;
        trash(path, self.owner_window).map_err(fail(path))?;
        self.done += 1;
        Ok(true)
    }

    // The start of every item: a chance to stop, and a progress report.
    fn begin(&mut self, path: &Path) -> Result<(), Stop> {
        self.check_cancel()?;
        let now = Instant::now();
        let due = self
            .last_progress
            .is_none_or(|last| now.duration_since(last) >= PROGRESS_INTERVAL);
        if due {
            self.last_progress = Some(now);
            (self.report)(OperationEvent::Progress {
                current: path.to_path_buf(),
                done: self.done,
            });
        }
        Ok(())
    }

    fn check_cancel(&self) -> Result<(), Stop> {
        match self.replies.try_recv() {
            Ok(Reply::Cancel) | Err(TryRecvError::Disconnected) => Err(Stop::Cancelled),
            Ok(_) | Err(TryRecvError::Empty) => Ok(()),
        }
    }
}

// The entries of a directory in name order, so that progress is predictable.
fn children(dir: &Path) -> Result<Vec<PathBuf>, Stop> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(fail(dir))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<_>>()
        .map_err(fail(dir))?;
    paths.sort();
    Ok(paths)
}

// Best effort: a file that could not take its time or mode is still copied.
fn copy_metadata(dst: &Path, meta: &Metadata) {
    if let Ok(modified) = meta.modified()
        && let Ok(file) = open_for_times(dst)
    {
        let _ = file.set_times(FileTimes::new().set_modified(modified));
    }
    let _ = fs::set_permissions(dst, meta.permissions());
}

#[cfg(unix)]
fn open_for_times(path: &Path) -> io::Result<File> {
    File::open(path)
}

#[cfg(windows)]
fn open_for_times(path: &Path) -> io::Result<File> {
    use std::os::windows::fs::OpenOptionsExt;

    // Directories can only be opened with backup semantics.
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    fs::OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
}

#[cfg(unix)]
fn make_symlink(target: &Path, _src: &Path, dst: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, dst)
}

#[cfg(windows)]
fn make_symlink(target: &Path, src: &Path, dst: &Path) -> io::Result<()> {
    // The kind of link is fixed on Windows; a broken link becomes a file link.
    if fs::metadata(src).is_ok_and(|meta| meta.is_dir()) {
        std::os::windows::fs::symlink_dir(target, dst)
    } else {
        std::os::windows::fs::symlink_file(target, dst)
    }
}

// Removes a file or a link, clearing the read-only attribute on Windows if
// that is what stands in the way.
fn remove_file(path: &Path) -> io::Result<()> {
    retry_writable(path, remove_file_or_link)
}

fn remove_dir(path: &Path) -> io::Result<()> {
    retry_writable(path, |path| fs::remove_dir(path))
}

#[cfg(windows)]
fn remove_file_or_link(path: &Path) -> io::Result<()> {
    // A link to a directory is a directory to the removal functions.
    let meta = fs::symlink_metadata(path)?;
    if meta.is_symlink() && meta.is_dir() {
        fs::remove_dir(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(not(windows))]
fn remove_file_or_link(path: &Path) -> io::Result<()> {
    fs::remove_file(path)
}

#[cfg(windows)]
fn retry_writable(path: &Path, remove: fn(&Path) -> io::Result<()>) -> io::Result<()> {
    match remove(path) {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            if let Ok(meta) = fs::symlink_metadata(path) {
                let mut permissions = meta.permissions();
                // Only the Windows read-only attribute is touched here; the
                // entry is about to be removed anyway.
                #[allow(clippy::permissions_set_readonly_false)]
                permissions.set_readonly(false);
                let _ = fs::set_permissions(path, permissions);
            }
            remove(path)
        }
        result => result,
    }
}

#[cfg(not(windows))]
fn retry_writable(path: &Path, remove: fn(&Path) -> io::Result<()>) -> io::Result<()> {
    remove(path)
}

fn fail(path: &Path) -> impl Fn(io::Error) -> Stop {
    let path = path.to_path_buf();
    move |error| Stop::Failed {
        path: path.clone(),
        error,
    }
}

fn failure(path: &Path, message: &str) -> Stop {
    Stop::Failed {
        path: path.to_path_buf(),
        error: io::Error::other(message),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::mpsc::{self, Sender};
    use std::time::SystemTime;

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn copies_files_and_directories_with_their_times() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        fs::create_dir_all(src.join("d").join("e")).unwrap();
        fs::create_dir(&dst).unwrap();
        fs::write(src.join("a"), b"hello").unwrap();
        fs::write(src.join("d").join("b"), b"x").unwrap();
        fs::write(src.join("d").join("e").join("c"), b"").unwrap();
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        for path in [src.join("a"), src.join("d").join("b"), src.join("d")] {
            open_for_times(&path)
                .unwrap()
                .set_times(FileTimes::new().set_modified(old))
                .unwrap();
        }

        let (outcome, events) = run(&copy(&[src.join("a"), src.join("d")], &dst), []);

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 5,
                skipped: 0
            }
        );
        assert_eq!(fs::read(dst.join("a")).unwrap(), b"hello");
        assert_eq!(fs::read(dst.join("d").join("b")).unwrap(), b"x");
        assert!(dst.join("d").join("e").join("c").is_file());
        assert_eq!(modified(&dst.join("a")), old);
        assert_eq!(modified(&dst.join("d").join("b")), old);
        assert_eq!(modified(&dst.join("d")), old);
        assert_eq!(
            events[0],
            OperationEvent::Progress {
                current: src.join("a"),
                done: 0
            }
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, OperationEvent::ItemDone { .. }))
                .collect::<Vec<_>>(),
            [
                &OperationEvent::ItemDone { index: 0 },
                &OperationEvent::ItemDone { index: 1 }
            ]
        );
        // The source is untouched.
        assert!(src.join("a").is_file());
    }

    #[cfg(unix)]
    #[test]
    fn copies_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        fs::create_dir_all(&src).unwrap();
        fs::create_dir(&dst).unwrap();
        fs::write(src.join("x"), b"#!/bin/sh").unwrap();
        fs::set_permissions(src.join("x"), fs::Permissions::from_mode(0o755)).unwrap();

        let (outcome, _) = run(&copy(&[src.join("x")], &dst), []);

        assert!(matches!(outcome, Outcome::Completed { .. }));
        assert_eq!(
            fs::metadata(dst.join("x")).unwrap().permissions().mode() & 0o777,
            0o755
        );
    }

    #[test]
    fn merges_into_an_existing_directory() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        fs::create_dir_all(src.join("d")).unwrap();
        fs::create_dir_all(dst.join("d")).unwrap();
        fs::write(src.join("d").join("new"), b"n").unwrap();
        fs::write(dst.join("d").join("old"), b"o").unwrap();

        let (outcome, _) = run(&copy(&[src.join("d")], &dst), []);

        assert!(matches!(outcome, Outcome::Completed { .. }));
        assert!(dst.join("d").join("new").is_file());
        assert!(dst.join("d").join("old").is_file());
    }

    #[test]
    fn a_conflict_is_asked_about_and_overwritten_or_skipped() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &["a", "b"], &["a", "b"]);

        let (outcome, events) = run(
            &copy(&[src.join("a"), src.join("b")], &dst),
            [Reply::Overwrite { all: false }, Reply::Skip { all: false }],
        );

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 1,
                skipped: 1
            }
        );
        assert_eq!(fs::read(dst.join("a")).unwrap(), b"src a");
        assert_eq!(fs::read(dst.join("b")).unwrap(), b"dst b");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, OperationEvent::Conflict { .. }))
                .count(),
            2
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, OperationEvent::ItemDone { .. }))
                .collect::<Vec<_>>(),
            [&OperationEvent::ItemDone { index: 0 }]
        );
    }

    #[test]
    fn apply_to_all_answers_the_conflicts_that_follow() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &["a", "b", "c"], &["a", "b", "c"]);

        let (outcome, events) = run(
            &copy(&[src.join("a"), src.join("b"), src.join("c")], &dst),
            [Reply::Skip { all: true }],
        );

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 0,
                skipped: 3
            }
        );
        assert_eq!(conflicts(&events), 1);
        assert_eq!(fs::read(dst.join("c")).unwrap(), b"dst c");

        let (outcome, events) = run(
            &copy(&[src.join("a"), src.join("b")], &dst),
            [Reply::Overwrite { all: true }],
        );
        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 2,
                skipped: 0
            }
        );
        assert_eq!(conflicts(&events), 1);
        assert_eq!(fs::read(dst.join("b")).unwrap(), b"src b");
    }

    #[test]
    fn renaming_a_conflict_writes_the_new_name_and_asks_again_when_it_exists() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &["a"], &["a", "a2"]);

        let (outcome, events) = run(
            &copy(&[src.join("a")], &dst),
            [
                Reply::Rename(OsString::from("a2")),
                Reply::Rename(OsString::from("a3")),
            ],
        );

        assert!(matches!(outcome, Outcome::Completed { done: 1, .. }));
        assert_eq!(fs::read(dst.join("a3")).unwrap(), b"src a");
        assert_eq!(fs::read(dst.join("a2")).unwrap(), b"dst a2");
        assert_eq!(
            events[2],
            OperationEvent::Conflict {
                source: src.join("a"),
                dest: dst.join("a2"),
            }
        );
    }

    #[test]
    fn aborting_a_conflict_cancels() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &["a", "b"], &["a"]);

        let (outcome, _) = run(
            &copy(&[src.join("a"), src.join("b")], &dst),
            [Reply::Cancel],
        );

        assert_eq!(outcome, Outcome::Cancelled { done: 0 });
        assert!(!dst.join("b").exists());
    }

    #[test]
    fn copying_a_file_onto_itself_is_an_error() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a"), b"a").unwrap();

        let (outcome, _) = run(
            &copy(&[dir.path().join("a")], dir.path()),
            [Reply::Overwrite { all: false }],
        );

        assert!(matches!(outcome, Outcome::Failed { .. }));
        assert_eq!(fs::read(dir.path().join("a")).unwrap(), b"a");
    }

    #[test]
    fn a_file_and_a_directory_of_the_same_name_are_an_error() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &["x"], &[]);
        fs::create_dir(dst.join("x")).unwrap();

        let (outcome, _) = run(&copy(&[src.join("x")], &dst), []);

        let Outcome::Failed { path, error, .. } = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert_eq!(path, src.join("x"));
        assert_eq!(
            error.to_string(),
            "the destination exists and is a directory"
        );
    }

    #[test]
    fn copying_a_directory_into_itself_is_an_error() {
        let dir = TempDir::new().unwrap();
        let d = dir.path().join("d");
        fs::create_dir_all(d.join("sub")).unwrap();

        let (outcome, _) = run(&copy(std::slice::from_ref(&d), &d.join("sub")), []);

        let Outcome::Failed { error, .. } = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert_eq!(error.to_string(), "the destination is inside the source");
        assert!(!d.join("sub").join("d").exists());
    }

    #[cfg(unix)]
    #[test]
    fn links_are_copied_as_links_and_deleted_without_their_targets() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &["real"], &[]);
        std::os::unix::fs::symlink("real", src.join("link")).unwrap();
        std::os::unix::fs::symlink("nowhere", src.join("broken")).unwrap();

        let (outcome, _) = run(&copy(&[src.join("link"), src.join("broken")], &dst), []);

        assert!(matches!(outcome, Outcome::Completed { done: 2, .. }));
        assert_eq!(fs::read_link(dst.join("link")).unwrap(), Path::new("real"));
        assert_eq!(
            fs::read_link(dst.join("broken")).unwrap(),
            Path::new("nowhere")
        );

        let (outcome, _) = run(&delete(&[src.join("link")]), []);

        assert!(matches!(outcome, Outcome::Completed { done: 1, .. }));
        assert!(!src.join("link").exists());
        assert!(src.join("real").is_file());
    }

    #[test]
    fn moves_by_renaming_and_merges_directories() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &["a"], &[]);
        fs::create_dir_all(src.join("d")).unwrap();
        fs::create_dir_all(dst.join("d")).unwrap();
        fs::write(src.join("d").join("x"), b"x").unwrap();
        fs::write(src.join("d").join("y"), b"src y").unwrap();
        fs::write(dst.join("d").join("y"), b"dst y").unwrap();

        let (outcome, events) = run(
            &mv(&[src.join("a"), src.join("d")], &dst),
            [Reply::Skip { all: false }],
        );

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 3,
                skipped: 1
            }
        );
        assert!(!src.join("a").exists());
        assert_eq!(fs::read(dst.join("a")).unwrap(), b"src a");
        assert!(dst.join("d").join("x").is_file());
        assert_eq!(fs::read(dst.join("d").join("y")).unwrap(), b"dst y");
        // The skipped file keeps its directory, and the item is not done.
        assert!(src.join("d").join("y").is_file());
        assert!(!src.join("d").join("x").exists());
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, OperationEvent::ItemDone { .. }))
                .collect::<Vec<_>>(),
            [&OperationEvent::ItemDone { index: 0 }]
        );
    }

    #[test]
    fn moving_over_a_file_overwrites_or_renames() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &["a", "b"], &["a", "b"]);

        let (outcome, _) = run(
            &mv(&[src.join("a"), src.join("b")], &dst),
            [
                Reply::Overwrite { all: false },
                Reply::Rename(OsString::from("b2")),
            ],
        );

        assert!(matches!(outcome, Outcome::Completed { done: 2, .. }));
        assert_eq!(fs::read(dst.join("a")).unwrap(), b"src a");
        assert_eq!(fs::read(dst.join("b")).unwrap(), b"dst b");
        assert_eq!(fs::read(dst.join("b2")).unwrap(), b"src b");
        assert!(!src.join("a").exists());
        assert!(!src.join("b").exists());
    }

    // A move across devices cannot be set up in a temporary directory, so
    // the copy-then-remove path is exercised directly.
    #[test]
    fn moving_by_copy_removes_each_source_after_it_is_copied() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &[], &["d"]);
        fs::create_dir_all(src.join("d").join("sub")).unwrap();
        fs::write(src.join("d").join("x"), b"x").unwrap();
        fs::write(src.join("d").join("sub").join("y"), b"y").unwrap();
        fs::remove_file(dst.join("d")).unwrap();
        let (_sender, replies) = mpsc::channel();
        let events = Mutex::new(Vec::new());
        let report = |event| events.lock().unwrap().push(event);
        let mut worker = Worker {
            owner_window: None,
            replies: &replies,
            report: &report,
            policy: None,
            done: 0,
            skipped: 0,
            last_progress: None,
        };

        let complete = worker
            .transfer_item(&src.join("d"), &dst.join("d"), Transfer::MoveByCopy)
            .ok();

        assert_eq!(complete, Some(true));
        assert_eq!(worker.done, 4);
        assert!(!src.join("d").exists());
        assert_eq!(fs::read(dst.join("d").join("x")).unwrap(), b"x");
        assert_eq!(fs::read(dst.join("d").join("sub").join("y")).unwrap(), b"y");
    }

    #[test]
    fn a_cancellation_during_a_file_removes_the_part_written() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &[], &[]);
        fs::write(src.join("big"), vec![7; CHUNK_SIZE * 3]).unwrap();
        let (sender, replies) = mpsc::channel();
        // The progress report comes after the cancellation check that starts
        // the item, so a cancellation sent then is seen once the file is open.
        let report = |event| {
            if matches!(event, OperationEvent::Progress { .. }) {
                sender.send(Reply::Cancel).unwrap();
            }
            if let OperationEvent::Finished(outcome) = event {
                assert_eq!(outcome, Outcome::Cancelled { done: 0 });
            }
        };

        run_operation(&copy(&[src.join("big")], &dst), None, &replies, &report);

        assert!(!dst.join("big").exists());
        assert!(src.join("big").is_file());
    }

    #[test]
    fn deletes_directories_recursively_and_read_only_files() {
        let dir = TempDir::new().unwrap();
        let d = dir.path().join("d");
        fs::create_dir_all(d.join("sub")).unwrap();
        fs::write(d.join("ro"), b"ro").unwrap();
        fs::write(d.join("sub").join("x"), b"x").unwrap();
        let mut permissions = fs::metadata(d.join("ro")).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(d.join("ro"), permissions).unwrap();

        let (outcome, events) = run(&delete(std::slice::from_ref(&d)), []);

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 4,
                skipped: 0
            }
        );
        assert!(!d.exists());
        assert_eq!(events.last(), Some(&OperationEvent::ItemDone { index: 0 }));
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_inside_a_copied_directory_fails_there() {
        let dir = TempDir::new().unwrap();
        let (src, dst) = two_dirs(&dir, &["a.txt"], &[]);
        let status = std::process::Command::new("mkfifo")
            .arg(src.join("fifo"))
            .status()
            .unwrap();
        assert!(status.success());

        let (outcome, _) = run(&copy(std::slice::from_ref(&src), &dst), []);

        let Outcome::Failed { path, error, .. } = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert_eq!(path, src.join("fifo"));
        assert_eq!(error.to_string(), "not a regular file");
        assert!(!dst.join("src").join("fifo").exists());
    }

    #[test]
    fn a_missing_source_fails_with_its_path() {
        let dir = TempDir::new().unwrap();

        let (outcome, _) = run(&delete(&[dir.path().join("missing")]), []);

        let Outcome::Failed { path, done, .. } = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert_eq!(path, dir.path().join("missing"));
        assert_eq!(done, 0);
    }

    // Real trash bins must not be touched by a test run.
    #[test]
    #[ignore = "moves a file into the user's trash"]
    fn trashes_items() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("t"), b"t").unwrap();

        let (outcome, _) = run(
            &OperationSpec {
                kind: OperationKind::Trash,
                sources: Sources::Paths(vec![dir.path().join("t")]),
                dest: None,
                output: None,
            },
            [],
        );

        assert!(matches!(outcome, Outcome::Completed { done: 1, .. }));
        assert!(!dir.path().join("t").exists());
    }

    // Runs an operation, answering each conflict with the next scripted reply.
    fn run(
        spec: &OperationSpec,
        script: impl IntoIterator<Item = Reply>,
    ) -> (Outcome, Vec<OperationEvent>) {
        let (sender, replies) = mpsc::channel();
        let script: Mutex<VecDeque<Reply>> = Mutex::new(script.into_iter().collect());
        let events = Mutex::new(Vec::new());
        let report = |event: OperationEvent| {
            if matches!(event, OperationEvent::Conflict { .. }) {
                answer(&sender, &script);
            }
            events.lock().unwrap().push(event);
        };
        run_operation(spec, None, &replies, &report);
        let mut events = events.into_inner().unwrap();
        let Some(OperationEvent::Finished(outcome)) = events.pop() else {
            panic!("the last event should be Finished");
        };
        (outcome, events)
    }

    fn answer(sender: &Sender<Reply>, script: &Mutex<VecDeque<Reply>>) {
        let reply = script
            .lock()
            .unwrap()
            .pop_front()
            .expect("a conflict without a scripted reply");
        sender.send(reply).unwrap();
    }

    fn conflicts(events: &[OperationEvent]) -> usize {
        events
            .iter()
            .filter(|event| matches!(event, OperationEvent::Conflict { .. }))
            .count()
    }

    // `src` and `dst` directories, each with the named files containing
    // `src <name>` or `dst <name>`.
    fn two_dirs(dir: &TempDir, src_files: &[&str], dst_files: &[&str]) -> (PathBuf, PathBuf) {
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        fs::create_dir(&src).unwrap();
        fs::create_dir(&dst).unwrap();
        for name in src_files {
            fs::write(src.join(name), format!("src {name}")).unwrap();
        }
        for name in dst_files {
            fs::write(dst.join(name), format!("dst {name}")).unwrap();
        }
        (src, dst)
    }

    fn copy(sources: &[PathBuf], dest: &Path) -> OperationSpec {
        OperationSpec {
            kind: OperationKind::Copy,
            sources: Sources::Paths(sources.to_vec()),
            dest: Some(dest.to_path_buf()),
            output: None,
        }
    }

    fn mv(sources: &[PathBuf], dest: &Path) -> OperationSpec {
        OperationSpec {
            kind: OperationKind::Move,
            sources: Sources::Paths(sources.to_vec()),
            dest: Some(dest.to_path_buf()),
            output: None,
        }
    }

    fn delete(sources: &[PathBuf]) -> OperationSpec {
        OperationSpec {
            kind: OperationKind::Delete,
            sources: Sources::Paths(sources.to_vec()),
            dest: None,
            output: None,
        }
    }

    fn modified(path: &Path) -> SystemTime {
        fs::metadata(path).unwrap().modified().unwrap()
    }
}
