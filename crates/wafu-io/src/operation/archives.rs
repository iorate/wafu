use std::fs::{self, File, FileTimes};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use wafu_core::{ArchiveEntries, ArchiveFormat, OperationEvent, OperationSpec, Reply};

use super::{
    CHUNK_SIZE, NOT_REGULAR, Step, Stop, Worker, children, fail, failure, is_within, open_for_times,
};
use crate::archive::{
    ArchiveEntry, ArchiveEntryKind, ArchiveError, ArchiveReader, ArchiveWriter, check_component,
    create_archive, open_archive,
};

const TEMP_PREFIX: &str = "wafu-";

// One archive being unpacked. Links are made after everything else so that
// no entry can be written through a link to somewhere outside the
// destination, and directory times and permissions are set once their
// contents are in.
struct Extraction {
    archive: PathBuf,
    reader: Box<dyn ArchiveReader>,
    password: Option<String>,
    dest: PathBuf,
    links: Vec<(PathBuf, String)>,
    dirs: Vec<(PathBuf, ArchiveEntry)>,
}

impl Extraction {
    fn open(
        archive: &Path,
        format: ArchiveFormat,
        password: Option<String>,
        dest: &Path,
    ) -> Result<Self, Stop> {
        let reader =
            open_archive(archive, format).map_err(|error| failure(archive, &error.to_string()))?;
        Ok(Self {
            archive: archive.to_path_buf(),
            reader,
            password,
            dest: dest.to_path_buf(),
            links: Vec::new(),
            dirs: Vec::new(),
        })
    }
}

impl Worker<'_> {
    pub(super) fn extract_archives(
        &mut self,
        archives: &[PathBuf],
        spec: &OperationSpec,
    ) -> Result<(), Stop> {
        for (index, archive) in archives.iter().enumerate() {
            let dest = spec
                .dest
                .as_deref()
                .ok_or_else(|| failure(archive, "no destination"))?;
            let format = ArchiveFormat::from_name(archive.file_name().unwrap_or_default())
                .unwrap_or(ArchiveFormat::Zip);
            let mut extraction = Extraction::open(archive, format, None, dest)?;
            let all: Vec<usize> = (0..extraction.reader.entries().len()).collect();
            let complete = self.extract_indices(&mut extraction, &all, 0)?;
            finish_extraction(extraction)?;
            if complete {
                (self.report)(OperationEvent::ItemDone { index });
            }
        }
        Ok(())
    }

    pub(super) fn extract_entries(
        &mut self,
        entries: &ArchiveEntries,
        dest: &Path,
    ) -> Result<(), Stop> {
        let mut extraction = Extraction::open(
            &entries.archive,
            entries.path.format,
            entries.password.clone(),
            dest,
        )?;
        let prefix = &entries.path.inner;
        let mut done = Vec::new();
        for (index, name) in entries.names.iter().enumerate() {
            let selected: Vec<usize> = extraction
                .reader
                .entries()
                .iter()
                .enumerate()
                .filter(|(_, entry)| {
                    entry.components.len() > prefix.len()
                        && entry.components[..prefix.len()] == **prefix
                        && entry.components[prefix.len()] == *name
                })
                .map(|(index, _)| index)
                .collect();
            if selected.is_empty() {
                let mut components = prefix.clone();
                components.push(name.clone());
                return Err(failure(
                    &shown(&extraction.archive, &components),
                    "not found in the archive",
                ));
            }
            if self.extract_indices(&mut extraction, &selected, prefix.len())? {
                done.push(index);
            }
        }
        finish_extraction(extraction)?;
        for index in done {
            (self.report)(OperationEvent::ItemDone { index });
        }
        Ok(())
    }

    pub(super) fn extract_to_temp(&mut self, entries: &ArchiveEntries) -> Result<(), Stop> {
        let dir = make_temp_dir().map_err(fail(&entries.archive))?;
        (self.report)(OperationEvent::TempDir(dir.clone()));
        self.extract_entries(entries, &dir)
    }

    pub(super) fn compress(&mut self, paths: &[PathBuf], spec: &OperationSpec) -> Result<(), Stop> {
        let Some(first) = paths.first() else {
            return Ok(());
        };
        let dest = spec
            .dest
            .as_deref()
            .ok_or_else(|| failure(first, "no destination"))?;
        let output = spec
            .output
            .as_ref()
            .ok_or_else(|| failure(first, "no output"))?;
        let out = dest.join(&output.name);
        if paths.iter().any(|path| is_within(&out, path)) {
            return Err(failure(&out, "the output is inside the source"));
        }
        // The first source stands for the operation in the conflict, since
        // the output is not a copy of anything in particular.
        let Some(out) = self.clear_destination(first, &out)? else {
            return Ok(());
        };
        let file = File::create(&out).map_err(fail(&out))?;
        let mut writer = create_archive(file, output.format, output.password.as_deref());
        match self.add_all(&mut writer, paths) {
            Ok(()) => match writer.finish() {
                Ok(()) => Ok(()),
                Err(error) => {
                    let _ = fs::remove_file(&out);
                    Err(Stop::Failed { path: out, error })
                }
            },
            Err(stop) => {
                drop(writer);
                let _ = fs::remove_file(&out);
                Err(stop)
            }
        }
    }

    fn extract_indices(
        &mut self,
        extraction: &mut Extraction,
        indices: &[usize],
        skip: usize,
    ) -> Step {
        let mut complete = true;
        for &index in indices {
            complete &= self.extract_one(extraction, index, skip)?;
        }
        Ok(complete)
    }

    // Writes one entry under the destination, with the first `skip`
    // components of its name left off.
    fn extract_one(&mut self, extraction: &mut Extraction, index: usize, skip: usize) -> Step {
        let entry = extraction.reader.entries()[index].clone();
        let shown = shown(&extraction.archive, &entry.components);
        self.begin(&shown)?;
        let relative = &entry.components[skip..];
        if relative.is_empty() || !relative.iter().all(|component| check_component(component)) {
            return Err(failure(&shown, "the entry name is unsafe"));
        }
        let dst = relative
            .iter()
            .fold(extraction.dest.clone(), |path, component| {
                path.join(component)
            });
        if !dst.starts_with(&extraction.dest) {
            return Err(failure(&shown, "the entry name is unsafe"));
        }
        match entry.kind {
            ArchiveEntryKind::Dir => {
                ensure_dir(&shown, &extraction.dest, &dst)?;
                extraction.dirs.push((dst, entry));
            }
            ArchiveEntryKind::File => {
                ensure_parent(&shown, &extraction.dest, &dst)?;
                self.unlock(extraction, index, &shown)?;
                let Some(dst) = self.clear_destination(&shown, &dst)? else {
                    return Ok(false);
                };
                let result = self.write_entry(extraction, index, &dst, &shown);
                if result.is_err() {
                    let _ = fs::remove_file(&dst);
                }
                result?;
                set_entry_metadata(&dst, &entry);
            }
            ArchiveEntryKind::Symlink => {
                self.unlock(extraction, index, &shown)?;
                let mut target = String::new();
                extraction
                    .reader
                    .open(index, extraction.password.as_deref())
                    .map_err(|error| failure(&shown, &error.to_string()))?
                    .read_to_string(&mut target)
                    .map_err(fail(&shown))?;
                ensure_parent(&shown, &extraction.dest, &dst)?;
                let Some(dst) = self.clear_destination(&shown, &dst)? else {
                    return Ok(false);
                };
                extraction.links.push((dst, target));
            }
        }
        self.done += 1;
        Ok(true)
    }

    // Makes sure the entry can be opened, asking for a password as long as
    // one is needed and the one given is wrong.
    fn unlock(
        &mut self,
        extraction: &mut Extraction,
        index: usize,
        shown: &Path,
    ) -> Result<(), Stop> {
        loop {
            let retry = match extraction
                .reader
                .open(index, extraction.password.as_deref())
            {
                Ok(_) => return Ok(()),
                Err(ArchiveError::PasswordRequired) => false,
                Err(ArchiveError::WrongPassword) => true,
                Err(error) => return Err(failure(shown, &error.to_string())),
            };
            extraction.password = Some(self.ask_password(&extraction.archive, retry)?);
        }
    }

    fn ask_password(&mut self, archive: &Path, retry: bool) -> Result<String, Stop> {
        (self.report)(OperationEvent::NeedPassword {
            archive: archive.to_path_buf(),
            retry,
        });
        loop {
            match self.replies.recv() {
                Ok(Reply::Password(password)) => return Ok(password),
                Ok(Reply::Cancel) | Err(_) => return Err(Stop::Cancelled),
                Ok(_) => {}
            }
        }
    }

    fn write_entry(
        &mut self,
        extraction: &mut Extraction,
        index: usize,
        dst: &Path,
        shown: &Path,
    ) -> Result<(), Stop> {
        let mut reader = extraction
            .reader
            .open(index, extraction.password.as_deref())
            .map_err(|error| failure(shown, &error.to_string()))?;
        let mut writer = File::create(dst).map_err(fail(shown))?;
        let mut buffer = vec![0; CHUNK_SIZE];
        loop {
            self.check_cancel()?;
            let read = reader.read(&mut buffer).map_err(fail(shown))?;
            if read == 0 {
                break;
            }
            writer.write_all(&buffer[..read]).map_err(fail(shown))?;
        }
        writer.flush().map_err(fail(shown))?;
        Ok(())
    }

    fn add_all(
        &mut self,
        writer: &mut Box<dyn ArchiveWriter>,
        paths: &[PathBuf],
    ) -> Result<(), Stop> {
        for (index, path) in paths.iter().enumerate() {
            let base = path.parent().unwrap_or(Path::new(""));
            self.add_path(writer, path, base)?;
            (self.report)(OperationEvent::ItemDone { index });
        }
        Ok(())
    }

    // Adds a path under its name relative to `base`, recursing into directories.
    fn add_path(
        &mut self,
        writer: &mut Box<dyn ArchiveWriter>,
        path: &Path,
        base: &Path,
    ) -> Result<(), Stop> {
        self.begin(path)?;
        let meta = fs::symlink_metadata(path).map_err(fail(path))?;
        let name = archive_name(path, base)?;
        let modified = meta.modified().ok();
        #[cfg(unix)]
        let mode = {
            use std::os::unix::fs::PermissionsExt;

            Some(meta.permissions().mode() & 0o777)
        };
        #[cfg(not(unix))]
        let mode = None;
        if meta.is_symlink() {
            let target = fs::read_link(path)
                .map_err(fail(path))?
                .to_string_lossy()
                .into_owned();
            #[cfg(windows)]
            let target = target.replace('\\', "/");
            writer
                .add_symlink(&name, &target, modified)
                .map_err(fail(path))?;
        } else if meta.is_dir() {
            writer.add_dir(&name, modified, mode).map_err(fail(path))?;
            for child in children(path)? {
                self.add_path(writer, &child, base)?;
            }
        } else if meta.is_file() {
            writer
                .start_file(&name, meta.len(), modified, mode)
                .map_err(fail(path))?;
            let mut reader = File::open(path).map_err(fail(path))?;
            let mut buffer = vec![0; CHUNK_SIZE];
            loop {
                self.check_cancel()?;
                let read = reader.read(&mut buffer).map_err(fail(path))?;
                if read == 0 {
                    break;
                }
                writer.write_all(&buffer[..read]).map_err(fail(path))?;
            }
        } else {
            return Err(failure(path, NOT_REGULAR));
        }
        self.done += 1;
        Ok(())
    }
}

fn finish_extraction(extraction: Extraction) -> Result<(), Stop> {
    for (dst, target) in extraction.links {
        make_link(&target, &dst).map_err(fail(&dst))?;
    }
    // Children were written after their directories, so deeper first.
    for (dir, entry) in extraction.dirs.iter().rev() {
        set_entry_metadata(dir, entry);
    }
    Ok(())
}

fn shown(archive: &Path, components: &[String]) -> PathBuf {
    components
        .iter()
        .fold(archive.to_path_buf(), |path, component| {
            path.join(component)
        })
}

fn archive_name(path: &Path, base: &Path) -> Result<String, Stop> {
    let relative = path
        .strip_prefix(base)
        .map_err(|_| failure(path, "not under its base"))?;
    Ok(relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/"))
}

fn ensure_dir(shown: &Path, root: &Path, dst: &Path) -> Result<(), Stop> {
    let relative = dst
        .strip_prefix(root)
        .map_err(|_| failure(shown, "the entry name is unsafe"))?;
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(existing) if existing.is_dir() => {}
            Ok(_) => {
                return Err(failure(
                    shown,
                    "the destination exists and is not a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&path).map_err(fail(shown))?;
            }
            Err(error) => {
                return Err(Stop::Failed {
                    path: shown.to_path_buf(),
                    error,
                });
            }
        }
    }
    Ok(())
}

// Archives need not list a directory before the files in it.
fn ensure_parent(shown: &Path, root: &Path, dst: &Path) -> Result<(), Stop> {
    match dst.parent() {
        Some(parent) => ensure_dir(shown, root, parent),
        None => Ok(()),
    }
}

// Best effort, as when copying.
fn set_entry_metadata(dst: &Path, entry: &ArchiveEntry) {
    if let Some(modified) = entry.modified {
        set_modified(dst, modified);
    }
    #[cfg(unix)]
    if let Some(mode) = entry.mode {
        use std::os::unix::fs::PermissionsExt;

        let _ = fs::set_permissions(dst, fs::Permissions::from_mode(mode & 0o777));
    }
}

fn set_modified(path: &Path, modified: SystemTime) {
    if let Ok(file) = open_for_times(path) {
        let _ = file.set_times(FileTimes::new().set_modified(modified));
    }
}

#[cfg(unix)]
fn make_link(target: &str, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, dst)
}

#[cfg(windows)]
fn make_link(target: &str, dst: &Path) -> std::io::Result<()> {
    let target = target.replace('/', "\\");
    let resolved = dst.parent().unwrap_or(Path::new("")).join(&target);
    if fs::metadata(resolved).is_ok_and(|meta| meta.is_dir()) {
        std::os::windows::fs::symlink_dir(target, dst)
    } else {
        std::os::windows::fs::symlink_file(target, dst)
    }
}

// A fresh directory under the system's temporary directory, left for the OS
// to clean up.
fn make_temp_dir() -> std::io::Result<PathBuf> {
    Ok(tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .tempdir_in(std::env::temp_dir())?
        .keep())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::io::Write;
    use std::sync::Mutex;
    use std::sync::mpsc::{self, Sender};
    use std::time::Duration;

    use tempfile::TempDir;
    use wafu_core::{ArchiveOutput, ArchivePath, OperationKind, Outcome, Sources};

    use super::super::run_operation;
    use super::*;
    use crate::archive::zip::ZipArchiveWriter;

    const MODIFIED: SystemTime = SystemTime::UNIX_EPOCH;

    #[test]
    fn extracts_the_named_entries_with_their_directories_and_times() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let modified = MODIFIED + Duration::from_secs(1_700_000_000);
        write_zip(&archive, None, |writer| {
            writer.add_dir("d", Some(modified), Some(0o755)).unwrap();
            writer
                .start_file("d/f.txt", 5, Some(modified), Some(0o644))
                .unwrap();
            writer.write_all(b"hello").unwrap();
            writer
                .start_file("d/sub/g.txt", 2, Some(modified), None)
                .unwrap();
            writer.write_all(b"hi").unwrap();
            writer
                .start_file("top.txt", 3, Some(modified), None)
                .unwrap();
            writer.write_all(b"top").unwrap();
            writer.start_file("other.txt", 1, None, None).unwrap();
            writer.write_all(b"x").unwrap();
        });

        let (outcome, events) = run(&copy_entries(&archive, &[], &["d", "top.txt"], &dest), []);

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 4,
                skipped: 0
            }
        );
        assert_eq!(fs::read(dest.join("d").join("f.txt")).unwrap(), b"hello");
        assert_eq!(
            fs::read(dest.join("d").join("sub").join("g.txt")).unwrap(),
            b"hi"
        );
        assert_eq!(fs::read(dest.join("top.txt")).unwrap(), b"top");
        assert!(!dest.join("other.txt").exists());
        assert_eq!(modified_of(&dest.join("top.txt")), modified);
        assert_eq!(modified_of(&dest.join("d").join("f.txt")), modified);
        assert_eq!(modified_of(&dest.join("d")), modified);
        assert_eq!(
            done_indices(&events),
            [0, 1],
            "every named entry is reported done"
        );
        assert!(matches!(
            events.first(),
            Some(OperationEvent::Progress { current, done: 0 }) if *current == archive.join("d")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn directory_permissions_are_restored_after_their_contents_are_written() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, None, |writer| {
            writer.add_dir("ro", None, Some(0o555)).unwrap();
            writer.start_file("ro/f.txt", 1, None, Some(0o644)).unwrap();
            writer.write_all(b"f").unwrap();
        });

        let (outcome, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 2,
                skipped: 0
            }
        );
        let ro = dest.join("ro");
        assert_eq!(fs::read(ro.join("f.txt")).unwrap(), b"f");
        assert_eq!(
            fs::metadata(&ro).unwrap().permissions().mode() & 0o777,
            0o555
        );
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn extract_unpacks_the_whole_archive_even_without_directory_entries() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, None, |writer| {
            writer.start_file("x/y/z.txt", 1, None, None).unwrap();
            writer.write_all(b"z").unwrap();
        });

        let (outcome, events) = run(&extract(&[archive], &dest), []);

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 1,
                skipped: 0
            }
        );
        assert_eq!(
            fs::read(dest.join("x").join("y").join("z.txt")).unwrap(),
            b"z"
        );
        assert_eq!(done_indices(&events), [0]);
    }

    #[test]
    fn conflicts_while_extracting_are_asked_about() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("a.txt"), b"old").unwrap();
        fs::write(dest.join("b.txt"), b"old").unwrap();
        fs::write(dest.join("c.txt"), b"old").unwrap();
        write_zip(&archive, None, |writer| {
            for name in ["a.txt", "b.txt", "c.txt"] {
                writer.start_file(name, 3, None, None).unwrap();
                writer.write_all(b"new").unwrap();
            }
        });

        let (outcome, events) = run(
            &copy_entries(&archive, &[], &["a.txt", "b.txt", "c.txt"], &dest),
            [
                Reply::Overwrite { all: false },
                Reply::Skip { all: false },
                Reply::Rename("c2.txt".into()),
            ],
        );

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 2,
                skipped: 1
            }
        );
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"new");
        assert_eq!(fs::read(dest.join("b.txt")).unwrap(), b"old");
        assert_eq!(fs::read(dest.join("c.txt")).unwrap(), b"old");
        assert_eq!(fs::read(dest.join("c2.txt")).unwrap(), b"new");
        assert_eq!(done_indices(&events), [0, 2]);
    }

    #[test]
    fn unsafe_names_fail_and_write_nothing_outside() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        for name in ["../evil.txt", "a/../../x.txt", "nul\0.txt"] {
            let archive = dir.path().join("bad.zip");
            fs::write(&archive, raw_zip(&[(name.as_bytes(), b"boom")])).unwrap();

            let (outcome, _) = run(&extract(std::slice::from_ref(&archive), &dest), []);

            assert!(
                matches!(&outcome, Outcome::Failed { error, .. } if error.to_string() == "the entry name is unsafe"),
                "{name}: {outcome:?}"
            );
        }
        assert!(!dir.path().join("evil.txt").exists());
        assert!(!dir.path().join("x.txt").exists());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn absolute_names_are_made_relative() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("abs.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        fs::write(&archive, raw_zip(&[(b"/abs.txt", b"a")])).unwrap();

        let (outcome, _) = run(&extract(std::slice::from_ref(&archive), &dest), []);

        assert!(matches!(outcome, Outcome::Completed { done: 1, .. }));
        assert_eq!(fs::read(dest.join("abs.txt")).unwrap(), b"a");
    }

    #[test]
    fn names_in_other_encodings_are_still_extracted() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("sjis.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        // "日本.txt" in Shift_JIS.
        let raw = [0x93, 0xfa, 0x96, 0x7b, b'.', b't', b'x', b't'];
        fs::write(&archive, raw_zip(&[(&raw, b"ok")])).unwrap();

        let (outcome, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 1,
                skipped: 0
            }
        );
        let names: Vec<String> = fs::read_dir(&dest)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 1);
        assert_eq!(
            Path::new(&names[0])
                .extension()
                .and_then(|ext| ext.to_str()),
            Some("txt")
        );
    }

    #[cfg(unix)]
    #[test]
    fn links_are_made_last_as_links() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("l.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, None, |writer| {
            writer.add_symlink("link", "f.txt", None).unwrap();
            writer.start_file("f.txt", 1, None, None).unwrap();
            writer.write_all(b"f").unwrap();
        });

        let (outcome, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 2,
                skipped: 0
            }
        );
        assert_eq!(
            fs::read_link(dest.join("link")).unwrap(),
            PathBuf::from("f.txt")
        );
        assert_eq!(fs::read(dest.join("link")).unwrap(), b"f");
    }

    #[cfg(unix)]
    #[test]
    fn copied_items_are_reported_done_after_their_links_are_made() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("l.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, None, |writer| {
            writer.add_symlink("link", "f.txt", None).unwrap();
            writer.start_file("f.txt", 1, None, None).unwrap();
            writer.write_all(b"f").unwrap();
        });
        let spec = copy_entries(&archive, &[], &["link", "f.txt"], &dest);
        let (_sender, replies) = mpsc::channel();
        let link_at_done = Mutex::new(Vec::new());
        let report = |event: OperationEvent| {
            if let OperationEvent::ItemDone { index } = event {
                let made = fs::symlink_metadata(dest.join("link")).is_ok();
                link_at_done.lock().unwrap().push((index, made));
            }
        };

        run_operation(&spec, None, &replies, &report);

        assert_eq!(link_at_done.into_inner().unwrap(), [(0, true), (1, true)]);
    }

    #[cfg(unix)]
    #[test]
    fn existing_links_in_the_destination_are_not_followed() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("l.zip");
        let dest = dir.path().join("out");
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir(&dest).unwrap();
        fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, dest.join("link")).unwrap();
        write_zip(&archive, None, |writer| {
            writer.start_file("link/sub/f.txt", 1, None, None).unwrap();
            writer.write_all(b"f").unwrap();
        });

        let (outcome, _) = run(&extract(&[archive], &dest), []);

        assert!(
            matches!(
                &outcome,
                Outcome::Failed { error, .. }
                    if error.to_string() == "the destination exists and is not a directory"
            ),
            "{outcome:?}"
        );
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    #[cfg(windows)]
    #[test]
    fn links_to_directories_are_made_as_directory_links_with_backslashes() {
        let dir = TempDir::new().unwrap();
        if std::os::windows::fs::symlink_file("x", dir.path().join("probe")).is_err() {
            return;
        }
        let archive = dir.path().join("l.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, None, |writer| {
            writer.add_symlink("link", "d/sub", None).unwrap();
            writer.start_file("d/sub/g.txt", 1, None, None).unwrap();
            writer.write_all(b"g").unwrap();
        });

        let (outcome, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 2,
                skipped: 0
            }
        );
        assert_eq!(
            fs::read_link(dest.join("link")).unwrap(),
            Path::new("d").join("sub")
        );
        assert_eq!(fs::read(dest.join("link").join("g.txt")).unwrap(), b"g");
    }

    #[test]
    fn a_password_is_asked_for_until_it_is_right() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("p.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, Some("secret"), |writer| {
            writer.start_file("a.txt", 1, None, None).unwrap();
            writer.write_all(b"a").unwrap();
            writer.start_file("b.txt", 1, None, None).unwrap();
            writer.write_all(b"b").unwrap();
        });

        let (outcome, events) = run(
            &copy_entries(&archive, &[], &["a.txt", "b.txt"], &dest),
            [
                Reply::Password("wrong".to_owned()),
                Reply::Password("secret".to_owned()),
            ],
        );

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 2,
                skipped: 0
            }
        );
        let asked: Vec<bool> = events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::NeedPassword { retry, .. } => Some(*retry),
                _ => None,
            })
            .collect();
        assert_eq!(asked, [false, true], "asked twice, then remembered");
        assert_eq!(fs::read(dest.join("b.txt")).unwrap(), b"b");

        let known = dir.path().join("known");
        fs::create_dir(&known).unwrap();
        let (outcome, events) = run(
            &copy_entries_with_password(&archive, &["a.txt"], &known, "secret"),
            [],
        );
        assert!(matches!(outcome, Outcome::Completed { done: 1, .. }));
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, OperationEvent::NeedPassword { .. }))
        );

        let cancelled = dir.path().join("cancelled");
        fs::create_dir(&cancelled).unwrap();
        let (outcome, _) = run(
            &copy_entries(&archive, &[], &["a.txt"], &cancelled),
            [Reply::Cancel],
        );
        assert_eq!(outcome, Outcome::Cancelled { done: 0 });
        assert_eq!(fs::read_dir(&cancelled).unwrap().count(), 0);
    }

    #[test]
    fn cancelling_the_password_after_choosing_to_overwrite_keeps_the_existing_file() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("p.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("a.txt"), b"old").unwrap();
        write_zip(&archive, Some("secret"), |writer| {
            writer.start_file("a.txt", 3, None, None).unwrap();
            writer.write_all(b"new").unwrap();
        });
        let spec = copy_entries(&archive, &[], &["a.txt"], &dest);
        let (sender, replies) = mpsc::channel();
        let events = Mutex::new(Vec::new());
        let report = |event: OperationEvent| {
            match event {
                OperationEvent::Conflict { .. } => {
                    sender.send(Reply::Overwrite { all: false }).unwrap();
                }
                OperationEvent::NeedPassword { .. } => sender.send(Reply::Cancel).unwrap(),
                _ => {}
            }
            events.lock().unwrap().push(event);
        };

        run_operation(&spec, None, &replies, &report);

        assert!(matches!(
            events.into_inner().unwrap().last(),
            Some(OperationEvent::Finished(Outcome::Cancelled { done: 0 }))
        ));
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"old");
    }

    #[test]
    fn a_name_that_is_not_in_the_archive_fails() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, None, |writer| {
            writer.start_file("a.txt", 0, None, None).unwrap();
        });

        let (outcome, _) = run(&copy_entries(&archive, &[], &["missing"], &dest), []);

        assert!(matches!(
            outcome,
            Outcome::Failed { path, error, .. }
                if path == archive.join("missing") && error.to_string() == "not found in the archive"
        ));
    }

    #[test]
    fn a_file_that_is_not_an_archive_fails_to_extract() {
        let dir = TempDir::new().unwrap();
        let text = dir.path().join("t.txt");
        fs::write(&text, b"text").unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        let (outcome, _) = run(&extract(std::slice::from_ref(&text), &dest), []);

        assert!(matches!(outcome, Outcome::Failed { path, .. } if path == text));
    }

    #[test]
    fn compress_packs_files_directories_and_links_relative_to_their_parent() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("d").join("sub")).unwrap();
        fs::write(src.join("d").join("f.txt"), b"hello").unwrap();
        fs::write(src.join("d").join("sub").join("g.txt"), b"g").unwrap();
        fs::write(src.join("top.txt"), b"top").unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        let (outcome, events) = run(
            &compress(&[src.join("d"), src.join("top.txt")], &dest, "x.zip", None),
            [],
        );

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 5,
                skipped: 0
            }
        );
        assert_eq!(done_indices(&events), [0, 1]);
        let mut reader = open_archive(&dest.join("x.zip"), ArchiveFormat::Zip).unwrap();
        let names: Vec<Vec<String>> = reader
            .entries()
            .iter()
            .map(|entry| entry.components.clone())
            .collect();
        assert_eq!(
            names,
            [
                vec!["d".to_owned()],
                vec!["d".to_owned(), "f.txt".to_owned()],
                vec!["d".to_owned(), "sub".to_owned()],
                vec!["d".to_owned(), "sub".to_owned(), "g.txt".to_owned()],
                vec!["top.txt".to_owned()],
            ]
        );
        let mut content = String::new();
        reader
            .open(1, None)
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "hello");
        assert!(reader.entries()[1].modified.is_some());
    }

    #[cfg(windows)]
    #[test]
    fn compress_records_link_targets_with_slashes() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("sub").join("f.txt"), b"f").unwrap();
        if std::os::windows::fs::symlink_file(Path::new("sub").join("f.txt"), src.join("link"))
            .is_err()
        {
            return;
        }
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        let (outcome, _) = run(&compress(&[src.join("link")], &dest, "x.zip", None), []);

        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 1,
                skipped: 0
            }
        );
        let mut reader = open_archive(&dest.join("x.zip"), ArchiveFormat::Zip).unwrap();
        let mut target = String::new();
        reader
            .open(0, None)
            .unwrap()
            .read_to_string(&mut target)
            .unwrap();
        assert_eq!(target, "sub/f.txt");
    }

    #[cfg(unix)]
    #[test]
    fn compress_fails_at_a_fifo() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("a.txt"), b"a").unwrap();
        let status = std::process::Command::new("mkfifo")
            .arg(src.join("fifo"))
            .status()
            .unwrap();
        assert!(status.success());
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        let (outcome, _) = run(
            &compress(std::slice::from_ref(&src), &dest, "x.zip", None),
            [],
        );

        let Outcome::Failed { path, error, .. } = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert_eq!(path, src.join("fifo"));
        assert_eq!(error.to_string(), "not a regular file");
    }

    #[test]
    fn compress_with_a_password_encrypts_every_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, b"a").unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        let (outcome, _) = run(&compress(&[src], &dest, "a.zip", Some("pw")), []);

        assert!(matches!(outcome, Outcome::Completed { done: 1, .. }));
        let mut reader = open_archive(&dest.join("a.zip"), ArchiveFormat::Zip).unwrap();
        assert!(reader.entries()[0].encrypted);
        assert!(matches!(
            reader.open(0, None),
            Err(ArchiveError::PasswordRequired)
        ));
        assert!(reader.open(0, Some("pw")).is_ok());
    }

    #[test]
    fn an_existing_output_is_a_conflict_and_an_output_inside_the_source_is_an_error() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, b"a").unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("a.zip"), b"old").unwrap();

        let (outcome, _) = run(
            &compress(std::slice::from_ref(&src), &dest, "a.zip", None),
            [Reply::Skip { all: false }],
        );
        assert_eq!(
            outcome,
            Outcome::Completed {
                done: 0,
                skipped: 1
            }
        );
        assert_eq!(fs::read(dest.join("a.zip")).unwrap(), b"old");

        let (outcome, _) = run(
            &compress(std::slice::from_ref(&src), &dest, "a.zip", None),
            [Reply::Overwrite { all: false }],
        );
        assert!(matches!(outcome, Outcome::Completed { done: 1, .. }));
        assert!(open_archive(&dest.join("a.zip"), ArchiveFormat::Zip).is_ok());

        let (outcome, _) = run(
            &compress(&[dir.path().to_path_buf()], &dest, "a.zip", None),
            [],
        );
        assert!(matches!(
            outcome,
            Outcome::Failed { error, .. } if error.to_string() == "the output is inside the source"
        ));
    }

    #[cfg(unix)]
    #[test]
    fn an_output_inside_the_source_through_a_link_is_an_error() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("a.txt"), b"a").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&src, &link).unwrap();

        let (outcome, _) = run(
            &compress(std::slice::from_ref(&src), &link, "a.zip", None),
            [],
        );

        assert!(matches!(
            outcome,
            Outcome::Failed { error, .. } if error.to_string() == "the output is inside the source"
        ));
        assert!(!src.join("a.zip").exists());
    }

    #[test]
    fn a_cancelled_compression_leaves_no_output() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, b"a").unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let (sender, replies) = mpsc::channel();
        sender.send(Reply::Cancel).unwrap();
        let events = Mutex::new(Vec::new());

        run_operation(
            &compress(&[src], &dest, "a.zip", None),
            None,
            &replies,
            &|event| {
                events.lock().unwrap().push(event);
            },
        );

        assert!(matches!(
            events.lock().unwrap().last(),
            Some(OperationEvent::Finished(Outcome::Cancelled { done: 0 }))
        ));
        assert!(!dest.join("a.zip").exists());
    }

    #[test]
    fn extract_to_temp_makes_a_fresh_directory_and_reports_it() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        write_zip(&archive, None, |writer| {
            writer.start_file("a.txt", 1, None, None).unwrap();
            writer.write_all(b"a").unwrap();
        });
        let spec = OperationSpec {
            kind: OperationKind::ExtractToTemp,
            sources: Sources::Entries(entries(&archive, &[], &["a.txt"], None)),
            dest: None,
            output: None,
        };

        let (outcome, events) = run(&spec, []);

        assert!(matches!(outcome, Outcome::Completed { done: 1, .. }));
        let Some(OperationEvent::TempDir(temp)) = events.first() else {
            panic!("expected TempDir first, got {events:?}");
        };
        assert_eq!(temp.parent(), Some(std::env::temp_dir().as_path()));
        assert!(
            temp.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with(TEMP_PREFIX)
        );
        assert_eq!(fs::read(temp.join("a.txt")).unwrap(), b"a");
        fs::remove_dir_all(temp).unwrap();
    }

    fn run(
        spec: &OperationSpec,
        script: impl IntoIterator<Item = Reply>,
    ) -> (Outcome, Vec<OperationEvent>) {
        let (sender, replies) = mpsc::channel();
        let script: Mutex<VecDeque<Reply>> = Mutex::new(script.into_iter().collect());
        let events = Mutex::new(Vec::new());
        let report = |event: OperationEvent| {
            if matches!(
                event,
                OperationEvent::Conflict { .. } | OperationEvent::NeedPassword { .. }
            ) {
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
            .expect("the script should have an answer");
        sender.send(reply).unwrap();
    }

    fn done_indices(events: &[OperationEvent]) -> Vec<usize> {
        events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::ItemDone { index } => Some(*index),
                _ => None,
            })
            .collect()
    }

    fn write_zip(path: &Path, password: Option<&str>, fill: impl FnOnce(&mut ZipArchiveWriter)) {
        let mut writer = ZipArchiveWriter::new(File::create(path).unwrap(), password);
        fill(&mut writer);
        Box::new(writer).finish().unwrap();
    }

    fn entries(
        archive: &Path,
        inner: &[&str],
        names: &[&str],
        password: Option<&str>,
    ) -> ArchiveEntries {
        ArchiveEntries {
            archive: archive.to_path_buf(),
            path: ArchivePath {
                format: ArchiveFormat::Zip,
                inner: inner.iter().map(|s| (*s).to_owned()).collect(),
            },
            names: names.iter().map(|s| (*s).to_owned()).collect(),
            password: password.map(ToOwned::to_owned),
        }
    }

    fn copy_entries(archive: &Path, inner: &[&str], names: &[&str], dest: &Path) -> OperationSpec {
        OperationSpec {
            kind: OperationKind::Copy,
            sources: Sources::Entries(entries(archive, inner, names, None)),
            dest: Some(dest.to_path_buf()),
            output: None,
        }
    }

    fn copy_entries_with_password(
        archive: &Path,
        names: &[&str],
        dest: &Path,
        password: &str,
    ) -> OperationSpec {
        OperationSpec {
            kind: OperationKind::Copy,
            sources: Sources::Entries(entries(archive, &[], names, Some(password))),
            dest: Some(dest.to_path_buf()),
            output: None,
        }
    }

    fn extract(archives: &[PathBuf], dest: &Path) -> OperationSpec {
        OperationSpec {
            kind: OperationKind::Extract,
            sources: Sources::Paths(archives.to_vec()),
            dest: Some(dest.to_path_buf()),
            output: None,
        }
    }

    fn compress(
        paths: &[PathBuf],
        dest: &Path,
        name: &str,
        password: Option<&str>,
    ) -> OperationSpec {
        OperationSpec {
            kind: OperationKind::Compress,
            sources: Sources::Paths(paths.to_vec()),
            dest: Some(dest.to_path_buf()),
            output: Some(ArchiveOutput {
                format: ArchiveFormat::Zip,
                name: name.into(),
                password: password.map(ToOwned::to_owned),
            }),
        }
    }

    fn modified_of(path: &Path) -> SystemTime {
        fs::metadata(path).unwrap().modified().unwrap()
    }

    // A stored zip written by hand, so that names the writer would refuse,
    // or in other encodings, can be tested.
    fn raw_zip(files: &[(&[u8], &[u8])]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let offset = u32::try_from(out.len()).unwrap();
            let crc = crc32(data);
            let size = u32::try_from(data.len()).unwrap();
            let name_len = u16::try_from(name.len()).unwrap();
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&20u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(&0x21u16.to_le_bytes());
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&size.to_le_bytes());
            out.extend_from_slice(&name_len.to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name);
            out.extend_from_slice(data);

            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&20u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0x21u16.to_le_bytes());
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&size.to_le_bytes());
            central.extend_from_slice(&name_len.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u16.to_le_bytes());
            central.extend_from_slice(&0u32.to_le_bytes());
            central.extend_from_slice(&offset.to_le_bytes());
            central.extend_from_slice(name);
        }
        let central_offset = u32::try_from(out.len()).unwrap();
        let central_size = u32::try_from(central.len()).unwrap();
        let count = u16::try_from(files.len()).unwrap();
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&count.to_le_bytes());
        out.extend_from_slice(&central_size.to_le_bytes());
        out.extend_from_slice(&central_offset.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xffff_ffffu32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xedb8_8320 & mask);
            }
        }
        !crc
    }
}
