use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, FileTimes};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::SystemTime;

use wafu_core::{
    ArchiveEntries, ArchiveFormat, ArchivePath, IoError, OperationEvent, OperationSpec,
    OuterArchive, Passwords, Reply,
};

use super::{
    CHUNK_SIZE, Destination, NOT_REGULAR, Step, Stop, Worker, children, fail, failure, is_within,
    open_for_times, refuse, refuse_io, remove_file, replace, temp_beside, write_replacing,
};
use crate::archive::{
    ArchiveEntry, ArchiveEntryKind, ArchiveError, ArchiveReader, ArchiveWriter, LevelError,
    NAME_LIMIT, NOT_FOUND, Stamp, UNSAFE_NAME, check_component, create_archive, decode_name,
    open_archive,
};

const TEMP_PREFIX: &str = "wafu-";
const LONG_TARGET: &str = "the link target is too long";
#[cfg(target_os = "macos")]
const QUARANTINE: &str = "com.apple.quarantine";

impl Worker<'_> {
    pub(super) fn extract_archives(
        &mut self,
        archives: &[PathBuf],
        spec: &OperationSpec,
        into_dir: bool,
    ) -> Result<(), Stop> {
        for (index, archive) in archives.iter().enumerate() {
            let name = archive.file_name().unwrap_or_default();
            let format = ArchiveFormat::from_name(name);
            self.extract_whole(index, archive, name, spec, into_dir, |_, dest| {
                Extraction::open(archive, format, dest)
            })?;
        }
        Ok(())
    }

    // Each entry is an archive inside the archive, unpacked as a file on disk
    // would be.
    pub(super) fn extract_nested_archives(
        &mut self,
        entries: &ArchiveEntries,
        spec: &OperationSpec,
        into_dir: bool,
    ) -> Result<(), Stop> {
        for (index, name) in entries.names.iter().enumerate() {
            let mut entry = entries.path.inner.clone();
            entry.push(name.clone());
            let mut outer = entries.path.outer.clone();
            outer.push(OuterArchive {
                format: entries.path.format,
                entry,
            });
            let path = ArchivePath {
                outer,
                format: ArchiveFormat::from_name(OsStr::new(name)),
                inner: Vec::new(),
            };
            let shown = nested_shown(&entries.archive, &path);
            self.extract_whole(
                index,
                &shown,
                OsStr::new(name),
                spec,
                into_dir,
                |worker, dest| {
                    worker.open_nested(&entries.archive, &path, entries.passwords.clone(), dest)
                },
            )?;
        }
        Ok(())
    }

    fn extract_whole(
        &mut self,
        index: usize,
        shown: &Path,
        name: &OsStr,
        spec: &OperationSpec,
        into_dir: bool,
        open: impl FnOnce(&Self, &Path) -> Result<Extraction, Stop>,
    ) -> Result<(), Stop> {
        let root = spec
            .dest
            .as_deref()
            .ok_or_else(|| failure(shown, "no destination"))?;
        // One destination serves the whole operation; naming the directory
        // here saves the core from passing one per archive.
        let dest = if into_dir {
            join_name(shown, root, ArchiveFormat::stem_of(name))?
        } else {
            root.to_path_buf()
        };
        let mut extraction = open(self, &dest)?;
        if into_dir {
            ensure_dir(
                shown,
                root,
                &dest,
                extraction.mark.as_deref(),
                &mut extraction.created,
            )
            .map_err(Stop::fatal)?;
        }
        let all: Vec<usize> = (0..extraction.reader.entries().len()).collect();
        let extracted = self.extract_indices(&mut extraction, &all, 0);
        let unlinked = self.finish_extraction(extraction, extracted.is_err());
        if extracted? && unlinked.is_empty() {
            (self.report)(OperationEvent::ItemDone { index });
        }
        Ok(())
    }

    pub(super) fn extract_entries(
        &mut self,
        entries: &ArchiveEntries,
        dest: &Path,
    ) -> Result<(), Stop> {
        let mut extraction = self.open_entries(entries, dest)?;
        let prefix = &entries.path.inner;
        let mut items: HashMap<&str, usize> = HashMap::new();
        for (item, name) in entries.names.iter().enumerate() {
            items.entry(name.as_str()).or_insert(item);
        }
        let owners: Vec<Option<usize>> = extraction
            .reader
            .entries()
            .iter()
            .map(|entry| match entry.components.get(prefix.len()) {
                Some(name) if entry.components[..prefix.len()] == **prefix => {
                    items.get(name.as_str()).copied()
                }
                _ => None,
            })
            .collect();
        let mut found = vec![false; entries.names.len()];
        for item in owners.iter().flatten() {
            found[*item] = true;
        }
        if let Some(missing) = entries
            .names
            .iter()
            .find(|name| !found[items[name.as_str()]])
        {
            let mut components = prefix.clone();
            components.push(missing.clone());
            return Err(failure(&shown(&extraction.archive, &components), NOT_FOUND));
        }
        // In archive order, since a tar is read front to back and going back
        // reads it again from the start.
        let mut complete = vec![true; entries.names.len()];
        let extracted = owners.iter().enumerate().try_for_each(|(index, item)| {
            if let Some(item) = *item {
                complete[item] &= self.extract_one(&mut extraction, index, prefix.len())?;
            }
            Ok(())
        });
        let unlinked = self.finish_extraction(extraction, extracted.is_err());
        extracted?;
        for index in unlinked {
            if let Some(item) = owners[index] {
                complete[item] = false;
            }
        }
        // Reported once the links are made, so that an item whose link could
        // not be made keeps its mark.
        for (index, name) in entries.names.iter().enumerate() {
            if complete[items[name.as_str()]] {
                (self.report)(OperationEvent::ItemDone { index });
            }
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
        let out = join_name(&dest.join(&output.name), dest, &output.name)?;
        if paths.iter().any(|path| is_within(&out, path)) {
            return Err(failure(&out, "the output is inside the source"));
        }
        // The first source stands for the operation in the conflict, since
        // the output is not a copy of anything in particular.
        let Some(out) = self.clear_destination(first, &out, None, paths)? else {
            return Ok(());
        };
        write_replacing(&out, &out.path, |path| {
            let file = File::create(path).map_err(fail(&out.path))?;
            let mut writer = create_archive(file, output.format, output.password.as_deref())
                .map_err(fail(&out.path))?;
            self.add_all(&mut writer, paths)?;
            writer.finish().map_err(fail(&out.path))
        })
    }

    fn open_entries(&self, entries: &ArchiveEntries, dest: &Path) -> Result<Extraction, Stop> {
        self.open_nested(
            &entries.archive,
            &entries.path,
            entries.passwords.clone(),
            dest,
        )
    }

    // Asks for the password of whichever level needs one, until all open.
    fn open_nested(
        &self,
        base: &Path,
        path: &ArchivePath,
        mut passwords: Passwords,
        dest: &Path,
    ) -> Result<Extraction, Stop> {
        let archive = nested_shown(base, path);
        // A cancel is taken off the replies when seen, so it is remembered
        // to tell the error it causes from others.
        let cancelled = Cell::new(false);
        let check = || {
            if !cancelled.get() && self.check_cancel().is_err() {
                cancelled.set(true);
            }
            cancelled.get()
        };
        let stamp = Stamp::of(base).map_err(fail(&archive))?;
        let innermost = path.outer.len();
        loop {
            let (level, retry) = match self.nested.open(base, stamp, path, &passwords, &check) {
                Ok(opened) => {
                    let password = passwords.get(innermost).map(ToOwned::to_owned);
                    return Ok(Extraction::new(
                        archive,
                        opened.reader,
                        innermost,
                        password,
                        read_mark(base),
                        dest,
                    ));
                }
                Err(_) if cancelled.get() => return Err(Stop::Cancelled),
                Err(LevelError {
                    level,
                    error: ArchiveError::PasswordRequired,
                }) => (level, false),
                Err(LevelError {
                    level,
                    error: ArchiveError::WrongPassword,
                }) => (level, true),
                Err(LevelError { error, .. }) => {
                    return Err(failure(&archive, &error.to_string()));
                }
            };
            let password = self.ask_password(&level_shown(base, path, level), level, retry)?;
            passwords.set(level, Some(password));
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

    // An entry that cannot be written is skipped, and the rest go on.
    fn extract_one(&mut self, extraction: &mut Extraction, index: usize, skip: usize) -> Step {
        match self.write_one(extraction, index, skip) {
            Err(Stop::Entry { path, error }) => {
                self.skip_entry(path, &error);
                Ok(false)
            }
            result => result,
        }
    }

    // Writes one entry under the destination, with the first `skip`
    // components of its name left off.
    fn write_one(&mut self, extraction: &mut Extraction, index: usize, skip: usize) -> Step {
        let entry = extraction.reader.entries()[index].clone();
        let shown = shown(&extraction.archive, &entry.components);
        self.begin(&shown)?;
        let relative = &entry.components[skip..];
        if relative.is_empty() {
            return Err(refuse(&shown, UNSAFE_NAME));
        }
        if let Err(reason) = relative
            .iter()
            .try_for_each(|component| check_component(component))
        {
            return Err(refuse(&shown, reason));
        }
        let dst = relative
            .iter()
            .fold(extraction.dest.clone(), |path, component| {
                path.join(component)
            });
        if !dst.starts_with(&extraction.dest) {
            return Err(refuse(&shown, UNSAFE_NAME));
        }
        // Probed before anything is made, since a name the file system cannot
        // hold fails the lookups on the way too, which would stop the rest.
        if let Err(error) = fs::symlink_metadata(&dst)
            && error.kind() == io::ErrorKind::InvalidFilename
        {
            return Err(refuse_io(&shown)(error));
        }
        match entry.kind {
            ArchiveEntryKind::Dir => {
                ensure_dir(
                    &shown,
                    &extraction.dest,
                    &dst,
                    extraction.mark.as_deref(),
                    &mut extraction.created,
                )?;
                // One that was already there is the user's, such as `~/.ssh`
                // under an archive's `.ssh/`, and keeps its permissions and
                // time.
                if extraction.created.contains(&dst) {
                    extraction.dirs.push((dst, entry));
                }
            }
            ArchiveEntryKind::File => {
                // A hard link is extracted as a copy. Its content comes before
                // it in a tar, so reading it there would start the stream over;
                // the file already written has it too, unless the mode it was
                // given keeps it from being read.
                let copy_of = extraction
                    .reader
                    .hard_link_target(index)
                    .and_then(|target| extraction.written.get(&target))
                    .and_then(|path| File::open(path).ok());
                if copy_of.is_none() {
                    self.unlock(extraction, index, &shown)?;
                }
                // Once the entry is known to be readable, so that one skipped
                // leaves no directories behind.
                ensure_parent(
                    &shown,
                    &extraction.dest,
                    &dst,
                    extraction.mark.as_deref(),
                    &mut extraction.created,
                )?;
                let Some(dst) = self.clear_destination(&shown, &dst, None, &[])? else {
                    return Ok(false);
                };
                write_replacing(&dst, &shown, |path| {
                    self.write_entry(extraction, index, copy_of, path, &shown)?;
                    // Before the metadata, since a read-only file refuses the
                    // mark and writing it on Windows changes the modification time.
                    mark_file(path, extraction.mark.as_deref());
                    set_entry_metadata(path, &entry, extraction.umask);
                    Ok(())
                })?;
                extraction.written.insert(index, dst.path);
            }
            ArchiveEntryKind::Symlink => {
                if entry.size > NAME_LIMIT {
                    return Err(refuse(&shown, LONG_TARGET));
                }
                self.unlock(extraction, index, &shown)?;
                let mut raw = Vec::new();
                extraction
                    .reader
                    .open(index, extraction.password.as_deref())
                    .map_err(|error| entry_error(&shown, error))?
                    .read_to_end(&mut raw)
                    .map_err(fail(&shown))?;
                let target = decode_name(&raw);
                ensure_parent(
                    &shown,
                    &extraction.dest,
                    &dst,
                    extraction.mark.as_deref(),
                    &mut extraction.created,
                )?;
                let Some(dst) = self.clear_destination(&shown, &dst, None, &[])? else {
                    return Ok(false);
                };
                // Counted once the link is made.
                extraction.links.push((dst, target, index, shown));
                return Ok(true);
            }
        }
        self.done += 1;
        Ok(true)
    }

    fn skip_entry(&mut self, path: PathBuf, error: &io::Error) {
        self.skipped += 1;
        (self.report)(OperationEvent::Skipped {
            path,
            error: IoError::new(error.to_string()),
        });
    }

    // Returns the entries whose links could not be made. None is made when the
    // extraction stopped early, so that the files they were to replace stay.
    fn finish_extraction(&mut self, extraction: Extraction, stopped: bool) -> Vec<usize> {
        let mut unlinked = Vec::new();
        let links = if stopped {
            Vec::new()
        } else {
            extraction.links
        };
        for (dst, target, index, shown) in links {
            let made = if dst.replace {
                temp_beside(&dst.path, |path| make_link(&target, path)).and_then(|temp| {
                    replace(&temp, &dst.path).inspect_err(|_| {
                        let _ = remove_file(&temp);
                    })
                })
            } else {
                make_link(&target, &dst.path)
            };
            match made {
                Ok(()) => self.done += 1,
                Err(error) => {
                    self.skip_entry(shown, &error);
                    unlinked.push(index);
                }
            }
        }
        // Children were written after their directories, so deeper first.
        for (dir, entry) in extraction.dirs.iter().rev() {
            set_entry_metadata(dir, entry, extraction.umask);
        }
        unlinked
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
                Err(error) => return Err(entry_error(shown, error)),
            };
            extraction.password =
                Some(self.ask_password(&extraction.archive, extraction.level, retry)?);
        }
    }

    fn ask_password(&self, archive: &Path, level: usize, retry: bool) -> Result<String, Stop> {
        (self.report)(OperationEvent::NeedPassword {
            archive: archive.to_path_buf(),
            level,
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
        copy_of: Option<File>,
        dst: &Path,
        shown: &Path,
    ) -> Result<(), Stop> {
        let mut reader: Box<dyn Read> = match copy_of {
            Some(file) => Box::new(file),
            None => extraction
                .reader
                .open(index, extraction.password.as_deref())
                .map_err(|error| entry_error(shown, error))?,
        };
        let mut writer = File::create(dst).map_err(|error| creation_error(shown, error))?;
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
            // Link targets are stored with `/`, the separator of the archive,
            // and turned back on extraction.
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
            let mut data = Cancellable {
                inner: File::open(path).map_err(fail(path))?,
                replies: self.replies,
            };
            writer
                .add_file(&name, meta.len(), modified, mode, &mut data)
                .map_err(|error| {
                    if error
                        .get_ref()
                        .is_some_and(|inner| inner.downcast_ref::<Cancelled>().is_some())
                    {
                        Stop::Cancelled
                    } else {
                        fail(path)(error)
                    }
                })?;
        } else {
            return Err(failure(path, NOT_REGULAR));
        }
        self.done += 1;
        Ok(())
    }
}

// A file being added to an archive. The writer pulls the content, so the
// cancellation is checked here, at each read, and comes back out of the
// writer as an error carrying `Cancelled`.
struct Cancellable<'a> {
    inner: File,
    replies: &'a Receiver<Reply>,
}

#[derive(Debug)]
struct Cancelled;

impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("cancelled")
    }
}

impl std::error::Error for Cancelled {}

impl Read for Cancellable<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self.replies.try_recv() {
            Ok(Reply::Cancel) | Err(TryRecvError::Disconnected) => {
                return Err(io::Error::other(Cancelled));
            }
            Ok(_) | Err(TryRecvError::Empty) => {}
        }
        self.inner.read(buf)
    }
}

// One archive being unpacked. Links are made after everything else so that
// no entry can be written through a link to somewhere outside the
// destination, and directory times and permissions are set once their
// contents are in.
struct Extraction {
    archive: PathBuf,
    reader: Box<dyn ArchiveReader>,
    // The level of the archive among those read to reach it, and its password.
    level: usize,
    password: Option<String>,
    // The mark of where the outermost archive came from, read from it since
    // a nested archive is read from a temporary file.
    mark: Option<Vec<u8>>,
    dest: PathBuf,
    // The link, its target, the entry's index, and its path as shown.
    links: Vec<(Destination, String, usize, PathBuf)>,
    dirs: Vec<(PathBuf, ArchiveEntry)>,
    created: HashSet<PathBuf>,
    written: HashMap<usize, PathBuf>,
    umask: u32,
}

impl Extraction {
    fn open(archive: &Path, format: Option<ArchiveFormat>, dest: &Path) -> Result<Self, Stop> {
        let reader =
            open_archive(archive, format).map_err(|error| failure(archive, &error.to_string()))?;
        Ok(Self::new(
            archive.to_path_buf(),
            reader,
            0,
            None,
            read_mark(archive),
            dest,
        ))
    }

    fn new(
        archive: PathBuf,
        reader: Box<dyn ArchiveReader>,
        level: usize,
        password: Option<String>,
        mark: Option<Vec<u8>>,
        dest: &Path,
    ) -> Self {
        Self {
            archive,
            reader,
            level,
            password,
            mark,
            dest: dest.to_path_buf(),
            links: Vec::new(),
            dirs: Vec::new(),
            created: HashSet::new(),
            written: HashMap::new(),
            umask: read_umask(),
        }
    }
}

// A nested archive is named by the path of the entries leading to it, as
// its entries are.
fn nested_shown(base: &Path, path: &ArchivePath) -> PathBuf {
    level_shown(base, path, path.outer.len())
}

fn level_shown(base: &Path, path: &ArchivePath, level: usize) -> PathBuf {
    path.outer[..level]
        .iter()
        .fold(base.to_path_buf(), |shown_path, outer| {
            shown(&shown_path, &outer.entry)
        })
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

// The name comes from the core or from an archive's name, and neither may
// lead out of `dir`.
fn join_name(shown: &Path, dir: &Path, name: &OsStr) -> Result<PathBuf, Stop> {
    check_component(&name.to_string_lossy()).map_err(|reason| failure(shown, reason))?;
    Ok(dir.join(name))
}

fn ensure_dir(
    shown: &Path,
    root: &Path,
    dst: &Path,
    mark: Option<&[u8]>,
    created: &mut HashSet<PathBuf>,
) -> Result<(), Stop> {
    let relative = dst
        .strip_prefix(root)
        .map_err(|_| refuse(shown, UNSAFE_NAME))?;
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component);
        match fs::symlink_metadata(&path) {
            Ok(existing) if existing.is_dir() => {}
            Ok(_) => {
                return Err(refuse(
                    shown,
                    "the destination exists and is not a directory",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&path).map_err(|error| creation_error(shown, error))?;
                mark_dir(&path, mark);
                created.insert(path.clone());
            }
            Err(error) => {
                return Err(fail(shown)(error));
            }
        }
    }
    Ok(())
}

// An entry that cannot be read as it is, such as one in a form wafu does not
// support, is skipped; a damaged stream stops the extraction.
fn entry_error(shown: &Path, error: ArchiveError) -> Stop {
    match error {
        ArchiveError::Invalid(message) => refuse(shown, &message),
        ArchiveError::Io(error) => fail(shown)(error),
        error => failure(shown, &error.to_string()),
    }
}

// Only a name the file system cannot hold is a matter of the one entry; a
// full disk or a denied permission stops the rest as well.
fn creation_error(shown: &Path, error: io::Error) -> Stop {
    if error.kind() == io::ErrorKind::InvalidFilename {
        refuse_io(shown)(error)
    } else {
        fail(shown)(error)
    }
}

// Archives need not list a directory before the files in it.
fn ensure_parent(
    shown: &Path,
    root: &Path,
    dst: &Path,
    mark: Option<&[u8]>,
    created: &mut HashSet<PathBuf>,
) -> Result<(), Stop> {
    match dst.parent() {
        Some(parent) => ensure_dir(shown, root, parent, mark, created),
        None => Ok(()),
    }
}

// Best effort, as when copying.
fn set_entry_metadata(dst: &Path, entry: &ArchiveEntry, umask: u32) {
    if let Some(modified) = entry.modified {
        set_modified(dst, modified);
    }
    if let Some(mode) = entry.mode {
        set_mode(dst, mode, umask);
    }
}

fn set_modified(path: &Path, modified: SystemTime) {
    if let Ok(file) = open_for_times(path) {
        let _ = file.set_times(FileTimes::new().set_modified(modified));
    }
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32, umask: u32) {
    use std::os::unix::fs::PermissionsExt;

    let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777 & !umask));
}

#[cfg(not(unix))]
fn set_mode(_: &Path, _: u32, _: u32) {}

// Read from a directory made with the default permissions, since setting the
// umask to learn it is unsafe and changes it for every thread meanwhile.
#[cfg(unix)]
fn read_umask() -> u32 {
    use std::os::unix::fs::PermissionsExt;

    tempfile::tempdir()
        .and_then(|dir| fs::metadata(dir.path()))
        .map_or(0o022, |meta| 0o777 & !meta.permissions().mode())
}

#[cfg(not(unix))]
fn read_umask() -> u32 {
    0
}

// Read through a link, as the archive itself is.
#[cfg(target_os = "macos")]
fn read_mark(archive: &Path) -> Option<Vec<u8>> {
    xattr::get_deref(archive, QUARANTINE).ok().flatten()
}

#[cfg(windows)]
fn read_mark(archive: &Path) -> Option<Vec<u8>> {
    fs::read(zone_identifier(archive)).ok()
}

#[cfg(not(any(target_os = "macos", windows)))]
fn read_mark(_: &Path) -> Option<Vec<u8>> {
    None
}

#[cfg(target_os = "macos")]
fn mark_file(path: &Path, mark: Option<&[u8]>) {
    if let Some(mark) = mark {
        let _ = xattr::set(path, QUARANTINE, mark);
    }
}

#[cfg(windows)]
fn mark_file(path: &Path, mark: Option<&[u8]>) {
    if let Some(mark) = mark {
        let _ = fs::write(zone_identifier(path), mark);
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
fn mark_file(_: &Path, _: Option<&[u8]>) {}

#[cfg(target_os = "macos")]
fn mark_dir(path: &Path, mark: Option<&[u8]>) {
    mark_file(path, mark);
}

// Windows marks only the files it extracts from a zip.
#[cfg(not(target_os = "macos"))]
fn mark_dir(_: &Path, _: Option<&[u8]>) {}

#[cfg(windows)]
fn zone_identifier(path: &Path) -> PathBuf {
    let mut stream = path.as_os_str().to_owned();
    stream.push(":Zone.Identifier");
    PathBuf::from(stream)
}

#[cfg(unix)]
fn make_link(target: &str, dst: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, dst)
}

#[cfg(windows)]
fn make_link(target: &str, dst: &Path) -> std::io::Result<()> {
    let target = target.replace('/', "\\");
    // The kind of link is fixed on Windows and the archive does not record
    // it, so it follows what the target is at this moment.
    if links_to_dir(dst.parent().unwrap_or(Path::new("")), &target) {
        std::os::windows::fs::symlink_dir(target, dst)
    } else {
        std::os::windows::fs::symlink_file(target, dst)
    }
}

// Whether `target`, as the target of a link in `dir`, is a directory. Looking
// a path up on Windows connects to the host it names over SMB and tries to
// sign in to it, and the target comes from the archive. So only a relative
// target is looked up, and it counts as a file once a link is met, even as
// its last component, since another link from the same archive may lead to
// such a host. So does a target with a name that Windows reads as a device.
#[cfg(any(windows, test))]
fn links_to_dir(dir: &Path, target: &str) -> bool {
    if !is_plain_relative(target) {
        return false;
    }
    let mut path = dir.to_path_buf();
    for component in target.split(['/', '\\']) {
        match component {
            "" | "." => {}
            ".." => {
                path.pop();
            }
            name => {
                path.push(name);
                // `is_dir` is false for a link to a directory.
                if check_component(name).is_err()
                    || !fs::symlink_metadata(&path).is_ok_and(|meta| meta.is_dir())
                {
                    return false;
                }
            }
        }
    }
    true
}

// Whether `target` has neither a root nor a drive on Windows, judged from the
// text so that it is the same on every OS. A colon is refused anywhere, so
// that a component such as `C:` cannot make `PathBuf::push` start over.
#[cfg(any(windows, test))]
fn is_plain_relative(target: &str) -> bool {
    !target.starts_with(['/', '\\']) && !target.contains(':')
}

// A fresh directory under the system's temporary directory, left for the OS
// to clean up. Not one fixed `wafu` directory, because the temporary directory
// is shared between users on Linux and another user may already own that name.
fn make_temp_dir() -> std::io::Result<PathBuf> {
    Ok(tempfile::Builder::new()
        .prefix(TEMP_PREFIX)
        .tempdir_in(std::env::temp_dir())?
        .keep())
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::mpsc::{self, Sender};
    use std::time::Duration;

    use tar::{EntryType, Header};
    use tempfile::TempDir;
    use wafu_core::{
        ArchiveOutput, ArchivePath, OperationKind, Outcome, OuterArchive, Passwords, Sources,
    };

    use super::super::run_operation;
    use super::*;
    use crate::archive::NestedArchives;
    use crate::archive::tar::header_of;
    use crate::archive::tests::{archive_bytes, encrypted_bytes, write_archive, write_encrypted};
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
                .add_file(
                    "d/f.txt",
                    5,
                    Some(modified),
                    Some(0o644),
                    &mut &b"hello"[..],
                )
                .unwrap();
            writer
                .add_file("d/sub/g.txt", 2, Some(modified), None, &mut &b"hi"[..])
                .unwrap();
            writer
                .add_file("top.txt", 3, Some(modified), None, &mut &b"top"[..])
                .unwrap();
            writer
                .add_file("other.txt", 1, None, None, &mut &b"x"[..])
                .unwrap();
        });

        let (outcome, done, events) =
            run(&copy_entries(&archive, &[], &["d", "top.txt"], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 4);
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

    #[test]
    fn named_entries_are_extracted_in_archive_order_and_reported_in_item_order() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.tar");
        let dest = dir.path().join("out");
        fs::create_dir_all(dest.join("d")).unwrap();
        for name in [
            Path::new("d").join("x"),
            "a.txt".into(),
            Path::new("d").join("y"),
            "b.txt".into(),
        ] {
            fs::write(dest.join(name), b"old").unwrap();
        }
        write_tar(
            &archive,
            &[
                ("d/x", EntryType::Regular, "", b"x"),
                ("a.txt", EntryType::Regular, "", b"a"),
                ("d/y", EntryType::Regular, "", b"y"),
                ("b.txt", EntryType::Regular, "", b"b"),
            ],
        );

        let (outcome, done, events) = run(
            &copy_entries(&archive, &[], &["b.txt", "d", "a.txt"], &dest),
            (0..4).map(|_| Reply::Overwrite { all: false }),
        );

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 4);
        let asked: Vec<PathBuf> = events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::Conflict { source, .. } => Some(source.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            asked,
            [
                archive.join("d").join("x"),
                archive.join("a.txt"),
                archive.join("d").join("y"),
                archive.join("b.txt"),
            ]
        );
        assert_eq!(done_indices(&events), [0, 1, 2]);
        assert_eq!(fs::read(dest.join("b.txt")).unwrap(), b"b");
        assert_eq!(fs::read(dest.join("d").join("y")).unwrap(), b"y");
    }

    #[test]
    fn hard_links_are_copied_from_the_extracted_file_without_reading_the_archive_again() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.tar");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("same1"), b"old").unwrap();
        write_tar(
            &archive,
            &[
                ("orig", EntryType::Regular, "", b"data"),
                ("mid", EntryType::Regular, "", b"m"),
                ("same1", EntryType::Link, "orig", b""),
                ("same2", EntryType::Link, "orig", b""),
            ],
        );
        let spec = extract(std::slice::from_ref(&archive), &dest);
        let (sender, replies) = mpsc::channel();
        // With the archive emptied, reading it from the start again would fail.
        // Removing it would not do: the reader keeps its handle.
        let report = |event: OperationEvent| {
            if let OperationEvent::Conflict { .. } = event {
                File::options()
                    .write(true)
                    .open(&archive)
                    .unwrap()
                    .set_len(0)
                    .unwrap();
                sender.send(Reply::Overwrite { all: false }).unwrap();
            }
        };
        let nested = NestedArchives::default();
        let mut worker = Worker {
            owner_window: None,
            nested: &nested,
            replies: &replies,
            report: &report,
            policy: None,
            done: 0,
            skipped: 0,
            last_progress: None,
        };

        let outcome = worker.run(&spec);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(worker.done, 4);
        assert_eq!(fs::read(dest.join("same1")).unwrap(), b"data");
        assert_eq!(fs::read(dest.join("same2")).unwrap(), b"data");
    }

    #[cfg(unix)]
    #[test]
    fn a_hard_link_to_a_file_extracted_unreadable_is_read_from_the_archive() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.tar");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        {
            let mut builder = tar::Builder::new(File::create(&archive).unwrap());
            let mut header = header_of(EntryType::Regular, 4);
            header.set_mode(0o200);
            builder
                .append_data(&mut header, "orig", &b"data"[..])
                .unwrap();
            builder
                .append_link(&mut header_of(EntryType::Link, 0), "same", "orig")
                .unwrap();
            builder.finish().unwrap();
        }

        let (outcome, done, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 2);
        for name in ["orig", "same"] {
            let path = dest.join(name);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            assert_eq!(fs::read(path).unwrap(), b"data");
        }
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
            writer
                .add_file("ro/f.txt", 1, None, Some(0o644), &mut &b"f"[..])
                .unwrap();
        });

        let (outcome, done, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 2);
        let ro = dest.join("ro");
        assert_eq!(fs::read(ro.join("f.txt")).unwrap(), b"f");
        assert_eq!(
            fs::metadata(&ro).unwrap().permissions().mode() & 0o777,
            0o555
        );
        fs::set_permissions(&ro, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn a_directory_listed_after_its_contents_still_gets_its_time() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let modified = MODIFIED + Duration::from_secs(1_700_000_000);
        write_zip(&archive, None, |writer| {
            writer
                .add_file("d/f.txt", 1, None, None, &mut &b"f"[..])
                .unwrap();
            writer.add_dir("d", Some(modified), None).unwrap();
        });

        let (outcome, _, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(modified_of(&dest.join("d")), modified);
    }

    #[cfg(unix)]
    #[test]
    fn a_directory_that_was_already_there_keeps_its_mode() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let private = dest.join("private");
        fs::create_dir(&private).unwrap();
        fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
        write_zip(&archive, None, |writer| {
            writer.add_dir("private", None, Some(0o777)).unwrap();
        });

        let (outcome, _, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(
            fs::metadata(&private).unwrap().permissions().mode() & 0o7777,
            0o700
        );
    }

    #[cfg(unix)]
    #[test]
    fn modes_from_the_archive_go_through_the_umask() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let probe = dir.path().join("probe");
        fs::create_dir(&probe).unwrap();
        let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o7777;
        write_zip(&archive, None, |writer| {
            writer.add_dir("new", None, Some(0o777)).unwrap();
            writer
                .add_file("new/run.sh", 1, None, Some(0o777), &mut &b"x"[..])
                .unwrap();
        });

        let (outcome, _, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(mode(&dest.join("new/run.sh")), mode(&probe));
        assert_eq!(mode(&dest.join("new")), mode(&probe));
    }

    #[test]
    fn extract_unpacks_the_whole_archive_even_without_directory_entries() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, None, |writer| {
            writer
                .add_file("x/y/z.txt", 1, None, None, &mut &b"z"[..])
                .unwrap();
        });

        let (outcome, done, events) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 1);
        assert_eq!(
            fs::read(dest.join("x").join("y").join("z.txt")).unwrap(),
            b"z"
        );
        assert_eq!(done_indices(&events), [0]);
    }

    #[test]
    fn a_leading_dot_entry_and_an_empty_name_in_a_tar_are_left_out() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.tar");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_tar(
            &archive,
            &[
                ("./", EntryType::Directory, "", b""),
                ("./a.txt", EntryType::Regular, "", b"a"),
                ("./d/", EntryType::Directory, "", b""),
                ("./d/b.txt", EntryType::Regular, "", b"b"),
                ("", EntryType::Regular, "", b"x"),
            ],
        );

        let listed: Vec<Vec<String>> = open_archive(&archive, None)
            .unwrap()
            .entries()
            .iter()
            .map(|entry| entry.components.clone())
            .collect();
        assert_eq!(listed, [vec!["a.txt"], vec!["d"], vec!["d", "b.txt"]]);

        let (outcome, done, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 3);
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"a");
        assert_eq!(fs::read(dest.join("d").join("b.txt")).unwrap(), b"b");
    }

    #[test]
    fn root_entries_in_a_zip_are_left_out() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        fs::write(&archive, raw_zip(&[(b"./", b""), (b"./a.txt", b"a")])).unwrap();

        let listed: Vec<Vec<String>> = open_archive(&archive, None)
            .unwrap()
            .entries()
            .iter()
            .map(|entry| entry.components.clone())
            .collect();
        assert_eq!(listed, [vec!["a.txt"]]);

        let (outcome, done, events) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 1);
        assert_eq!(done_indices(&events), [0]);
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"a");
    }

    #[test]
    fn a_later_zip_entry_replaces_an_earlier_one_spelled_differently() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        fs::write(
            &archive,
            raw_zip(&[(b"a.txt", b"one"), (b"b.txt", b"b"), (b"./a.txt", b"two")]),
        )
        .unwrap();

        let reader = open_archive(&archive, None).unwrap();
        let listed: Vec<Vec<String>> = reader
            .entries()
            .iter()
            .map(|entry| entry.components.clone())
            .collect();
        assert_eq!(listed, [vec!["b.txt"], vec!["a.txt"]]);
        drop(reader);

        let (outcome, done, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 2);
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"two");
        assert_eq!(fs::read(dest.join("b.txt")).unwrap(), b"b");
    }

    #[test]
    fn a_tar_file_entry_whose_name_ends_with_a_slash_is_a_directory() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.tar");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_tar(
            &archive,
            &[
                ("d/", EntryType::Regular, "", b""),
                ("d/x", EntryType::Regular, "", b"x"),
            ],
        );

        let (outcome, done, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 2);
        assert_eq!(fs::read(dest.join("d").join("x")).unwrap(), b"x");
    }

    #[test]
    fn extract_into_dirs_unpacks_each_archive_into_a_directory_named_after_it() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.zip");
        let b = dir.path().join("b.jar");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&a, None, |writer| {
            writer
                .add_file("x.txt", 1, None, None, &mut &b"x"[..])
                .unwrap();
        });
        write_zip(&b, None, |writer| {
            writer
                .add_file("y.txt", 1, None, None, &mut &b"y"[..])
                .unwrap();
        });

        let (outcome, done, events) = run(&extract_into_dirs(&[a, b], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 2);
        assert_eq!(fs::read(dest.join("a").join("x.txt")).unwrap(), b"x");
        assert_eq!(fs::read(dest.join("b").join("y.txt")).unwrap(), b"y");
        assert_eq!(done_indices(&events), [0, 1]);
    }

    #[test]
    fn a_tar_gz_round_trips_and_its_directory_drops_both_extensions() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("d")).unwrap();
        fs::write(src.join("d").join("f.txt"), b"hello").unwrap();
        let dest = dir.path().join("dest");
        fs::create_dir(&dest).unwrap();
        let spec = compress(&[src.join("d")], &dest, "x.tar.gz", None);

        let (outcome, done, _) = run(&spec, []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 2);
        let archive = dest.join("x.tar.gz");
        assert_eq!(
            fs::read(&archive).unwrap()[..2],
            [0x1f, 0x8b],
            "a gzip stream"
        );

        let out = dir.path().join("out");
        fs::create_dir(&out).unwrap();
        let (outcome, _, _) = run(&extract_into_dirs(std::slice::from_ref(&archive), &out), []);

        assert!(matches!(outcome, Outcome::Completed { .. }));
        assert_eq!(
            fs::read(out.join("x").join("d").join("f.txt")).unwrap(),
            b"hello"
        );
    }

    #[test]
    fn extract_into_dirs_makes_the_directory_even_for_an_empty_archive() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, None, |_| {});

        let (outcome, done, _) = run(&extract_into_dirs(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 0);
        assert!(dest.join("a").is_dir());
    }

    #[test]
    fn extract_into_dirs_reuses_an_existing_directory_but_not_a_file() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a.zip");
        let b = dir.path().join("b.zip");
        let dest = dir.path().join("out");
        fs::create_dir_all(dest.join("a")).unwrap();
        fs::write(dest.join("a").join("old.txt"), b"old").unwrap();
        fs::write(dest.join("b"), b"file").unwrap();
        for archive in [&a, &b] {
            write_zip(archive, None, |writer| {
                writer
                    .add_file("x.txt", 1, None, None, &mut &b"x"[..])
                    .unwrap();
            });
        }

        let (outcome, _, events) = run(&extract_into_dirs(&[a, b.clone()], &dest), []);

        assert!(matches!(outcome, Outcome::Failed { path, .. } if path == b));
        assert_eq!(fs::read(dest.join("a").join("old.txt")).unwrap(), b"old");
        assert_eq!(fs::read(dest.join("a").join("x.txt")).unwrap(), b"x");
        assert_eq!(fs::read(dest.join("b")).unwrap(), b"file");
        assert_eq!(done_indices(&events), [0]);
    }

    #[test]
    fn extract_into_dirs_fails_an_archive_whose_name_makes_no_directory_below() {
        let dir = TempDir::new().unwrap();
        let parent = dir.path().join("parent");
        let dest = parent.join("out");
        fs::create_dir_all(&dest).unwrap();
        let archive = dir.path().join("...zip");
        write_archive(&archive, ArchiveFormat::Zip, &[("f.txt", b"f")]);

        let (outcome, _, _) = run(
            &extract_into_dirs(std::slice::from_ref(&archive), &dest),
            [],
        );

        assert!(
            matches!(&outcome, Outcome::Failed { path, .. } if *path == archive),
            "{outcome:?}"
        );
        assert!(!parent.join("f.txt").exists());
        assert!(!dest.join("f.txt").exists());
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
                writer
                    .add_file(name, 3, None, None, &mut &b"new"[..])
                    .unwrap();
            }
        });

        let (outcome, done, events) = run(
            &copy_entries(&archive, &[], &["a.txt", "b.txt", "c.txt"], &dest),
            [
                Reply::Overwrite { all: false },
                Reply::Skip { all: false },
                Reply::Rename("c2.txt".into()),
            ],
        );

        assert_eq!(outcome, Outcome::Completed { skipped: 1 });
        assert_eq!(done, 2);
        assert_eq!(fs::read(dest.join("a.txt")).unwrap(), b"new");
        assert_eq!(fs::read(dest.join("b.txt")).unwrap(), b"old");
        assert_eq!(fs::read(dest.join("c.txt")).unwrap(), b"old");
        assert_eq!(fs::read(dest.join("c2.txt")).unwrap(), b"new");
        assert_eq!(done_indices(&events), [0, 2]);
    }

    #[test]
    fn unsafe_names_are_skipped_and_write_nothing_outside() {
        let dir = TempDir::new().unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        for name in ["../evil.txt", "a/../../x.txt", "nul\0.txt"] {
            let archive = dir.path().join("bad.zip");
            fs::write(
                &archive,
                raw_zip(&[(name.as_bytes(), b"boom"), (b"good.txt", b"good")]),
            )
            .unwrap();

            let (outcome, done, events) = run(&extract(std::slice::from_ref(&archive), &dest), []);

            assert_eq!(outcome, Outcome::Completed { skipped: 1 }, "{name}");
            assert_eq!(done, 1);
            assert_eq!(
                skipped_errors(&events),
                ["the entry name is unsafe"],
                "{name}"
            );
            assert_eq!(done_indices(&events), Vec::<usize>::new(), "{name}");
            fs::remove_file(dest.join("good.txt")).unwrap();
        }
        assert!(!dir.path().join("evil.txt").exists());
        assert!(!dir.path().join("x.txt").exists());
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn tar_entries_that_cannot_be_read_are_skipped_without_their_directories() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.tar");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        {
            let mut builder = tar::Builder::new(File::create(&archive).unwrap());
            builder
                .append_pax_extensions([
                    ("GNU.sparse.major", &b"1"[..]),
                    ("GNU.sparse.minor", b"0"),
                    ("GNU.sparse.name", b"d/holes.bin"),
                    ("GNU.sparse.realsize", b"1048576"),
                ])
                .unwrap();
            let map = b"1\n0\n5\n";
            builder
                .append_data(
                    &mut header_of(EntryType::Regular, map.len() as u64),
                    "GNUSparseFile.0/holes.bin",
                    &map[..],
                )
                .unwrap();
            builder
                .append_data(&mut header_of(EntryType::new(b'M'), 1), "big", &b"x"[..])
                .unwrap();
            builder
                .append_link(&mut header_of(EntryType::Link, 0), "e/lost", "gone")
                .unwrap();
            builder
                .append_data(&mut header_of(EntryType::Regular, 2), "ok.txt", &b"ok"[..])
                .unwrap();
            builder.finish().unwrap();
        }

        let (outcome, done, events) = run(&extract(std::slice::from_ref(&archive), &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 3 });
        assert_eq!(done, 1);
        let skipped: Vec<(PathBuf, String)> = events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::Skipped { path, error } => Some((path.clone(), error.to_string())),
                _ => None,
            })
            .collect();
        assert_eq!(
            skipped,
            [
                (
                    archive.join("d").join("holes.bin"),
                    "sparse entries are not supported".to_owned()
                ),
                (
                    archive.join("big"),
                    "multi-volume archives are not supported".to_owned()
                ),
                (
                    archive.join("e").join("lost"),
                    "the target of the hard link is not in the archive".to_owned()
                ),
            ]
        );
        assert_eq!(fs::read(dest.join("ok.txt")).unwrap(), b"ok");
        assert!(!dest.join("d").exists());
        assert!(!dest.join("big").exists());
        assert!(!dest.join("e").exists());
    }

    #[test]
    fn zip_entries_in_a_compression_method_wafu_cannot_read_are_skipped() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let mut zip = raw_zip(&[(b"d/lzma.bin", b"x"), (b"ok.txt", b"ok")]);
        // LZMA, in the local and the central header of the first entry.
        zip[8..10].copy_from_slice(&14u16.to_le_bytes());
        let central = zip
            .windows(4)
            .position(|window| window == 0x0201_4b50u32.to_le_bytes())
            .unwrap();
        zip[central + 10..central + 12].copy_from_slice(&14u16.to_le_bytes());
        fs::write(&archive, zip).unwrap();

        let (outcome, done, events) = run(&extract(std::slice::from_ref(&archive), &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 1 });
        assert_eq!(done, 1);
        assert_eq!(
            skipped_errors(&events),
            ["the compression method is not supported"]
        );
        assert_eq!(fs::read(dest.join("ok.txt")).unwrap(), b"ok");
        assert!(!dest.join("d").exists());
    }

    #[test]
    fn a_file_entry_where_a_directory_exists_is_skipped() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir_all(dest.join("d")).unwrap();
        write_zip(&archive, None, |writer| {
            writer.add_file("d", 1, None, None, &mut &b"d"[..]).unwrap();
            writer
                .add_file("e.txt", 1, None, None, &mut &b"e"[..])
                .unwrap();
        });

        let (outcome, done, events) = run(&copy_entries(&archive, &[], &["d", "e.txt"], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 1 });
        assert_eq!(done, 1);
        assert_eq!(
            skipped_errors(&events),
            ["the destination exists and is a directory"]
        );
        assert_eq!(done_indices(&events), [1]);
        assert!(dest.join("d").is_dir());
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
            writer
                .add_file("f.txt", 1, None, None, &mut &b"f"[..])
                .unwrap();
        });

        let (outcome, done, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 2);
        assert_eq!(
            fs::read_link(dest.join("link")).unwrap(),
            PathBuf::from("f.txt")
        );
        assert_eq!(fs::read(dest.join("link")).unwrap(), b"f");
    }

    #[cfg(unix)]
    #[test]
    fn a_link_entry_overwriting_a_file_takes_its_place() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.tar");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("l"), b"old l").unwrap();
        write_tar(
            &archive,
            &[
                ("l", EntryType::Symlink, "b.txt", b""),
                ("b.txt", EntryType::Regular, "", b"new"),
            ],
        );

        let (outcome, done, _) = run(
            &extract(&[archive], &dest),
            [Reply::Overwrite { all: false }],
        );

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 2);
        assert_eq!(
            fs::read_link(dest.join("l")).unwrap(),
            PathBuf::from("b.txt")
        );
        assert_eq!(fs::read(dest.join("l")).unwrap(), b"new");
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 2);
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
            writer
                .add_file("f.txt", 1, None, None, &mut &b"f"[..])
                .unwrap();
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

        run_operation(&spec, None, &NestedArchives::default(), &replies, &report);

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
            writer
                .add_file("link/sub/f.txt", 1, None, None, &mut &b"f"[..])
                .unwrap();
        });

        let (outcome, done, events) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 1 });
        assert_eq!(done, 0);
        assert_eq!(
            skipped_errors(&events),
            ["the destination exists and is not a directory"]
        );
        assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
    }

    #[test]
    fn a_file_a_link_entry_was_to_replace_stays_when_a_later_conflict_cancels() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.tar");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        fs::write(dest.join("l"), b"old l").unwrap();
        fs::write(dest.join("b.txt"), b"old b").unwrap();
        write_tar(
            &archive,
            &[
                ("l", EntryType::Symlink, "b.txt", b""),
                ("b.txt", EntryType::Regular, "", b"new"),
            ],
        );

        let (outcome, _, _) = run(
            &extract(&[archive], &dest),
            [Reply::Overwrite { all: false }, Reply::Cancel],
        );

        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(fs::read(dest.join("l")).unwrap(), b"old l");
        assert_eq!(fs::read(dest.join("b.txt")).unwrap(), b"old b");
    }

    #[cfg(unix)]
    #[test]
    fn a_zip_link_whose_target_is_not_utf8_does_not_stop_the_extraction() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("l.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip_with_link(&archive, &mut &b"caf\xe9"[..]);

        let (outcome, _, events) = run(&extract(std::slice::from_ref(&archive), &dest), []);

        assert!(
            matches!(outcome, Outcome::Completed { .. }),
            "{outcome:?}, skipped {:?}",
            skipped_errors(&events)
        );
        assert!(fs::symlink_metadata(dest.join("l")).unwrap().is_symlink());
        assert_eq!(fs::read(dest.join("f.txt")).unwrap(), b"f");
    }

    #[cfg(unix)]
    #[test]
    fn a_zip_link_whose_target_is_longer_than_any_path_is_skipped_without_asking_the_os() {
        const LEN: u64 = NAME_LIMIT + 1;
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("l.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip_with_link(&archive, &mut io::repeat(b'a').take(LEN));

        let (outcome, _, events) = run(&extract(std::slice::from_ref(&archive), &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 1 });
        assert_eq!(fs::read(dest.join("f.txt")).unwrap(), b"f");
        assert_eq!(skipped_errors(&events), [LONG_TARGET]);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_that_cannot_be_made_is_skipped_and_directory_times_are_still_set() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("l.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let modified = MODIFIED + Duration::from_secs(1_700_000_000);
        write_zip(&archive, None, |writer| {
            writer.add_dir("d", Some(modified), None).unwrap();
            writer.add_symlink("d/l", "one", None).unwrap();
            // Its directory takes the place of the link.
            writer
                .add_file("d/l/x", 1, None, None, &mut &b"x"[..])
                .unwrap();
        });

        let (outcome, done, events) = run(&copy_entries(&archive, &[], &["d"], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 1 });
        assert_eq!(done, 2);
        assert_eq!(skipped_errors(&events).len(), 1);
        assert_eq!(done_indices(&events), Vec::<usize>::new());
        assert_eq!(fs::read(dest.join("d").join("l").join("x")).unwrap(), b"x");
        assert_eq!(modified_of(&dest.join("d")), modified);
    }

    #[test]
    fn an_entry_whose_name_is_too_long_is_skipped_and_the_rest_are_extracted() {
        for (case, long) in [
            ("file", "n".repeat(256)),
            ("dir", "n".repeat(256) + "/f.txt"),
        ] {
            let dir = TempDir::new().unwrap();
            let archive = dir.path().join("a.tar");
            let dest = dir.path().join("out");
            fs::create_dir(&dest).unwrap();
            let mut builder = tar::Builder::new(File::create(&archive).unwrap());
            for name in [long.as_str(), "last.txt"] {
                let mut header = Header::new_gnu();
                header.set_entry_type(EntryType::Regular);
                header.set_size(1);
                header.set_mode(0o644);
                builder.append_data(&mut header, name, &b"x"[..]).unwrap();
            }
            builder.into_inner().unwrap().flush().unwrap();

            let (outcome, _, _) = run(&extract(&[archive], &dest), []);

            assert_eq!(outcome, Outcome::Completed { skipped: 1 }, "{case}");
            assert!(dest.join("last.txt").exists(), "{case}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn names_windows_cannot_create_are_skipped() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        fs::write(
            &archive,
            raw_zip(&[
                (b"a:b", b"1"),
                (b"con", b"2"),
                (b"x.", b"3"),
                (b"ok.txt", b"4"),
            ]),
        )
        .unwrap();

        let (outcome, done, events) = run(&extract(std::slice::from_ref(&archive), &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 3 });
        assert_eq!(done, 1);
        assert_eq!(
            skipped_errors(&events),
            [
                "the entry name is unsafe",
                "the entry name is reserved for a device on Windows",
                "the entry name ends with a dot or a space, which Windows does not allow",
            ]
        );
        let written: Vec<_> = fs::read_dir(&dest)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(written, ["ok.txt"]);
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
            writer
                .add_file("d/sub/g.txt", 1, None, None, &mut &b"g"[..])
                .unwrap();
        });

        let (outcome, done, _) = run(&extract(&[archive], &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 2);
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
            writer
                .add_file("a.txt", 1, None, None, &mut &b"a"[..])
                .unwrap();
            writer
                .add_file("b.txt", 1, None, None, &mut &b"b"[..])
                .unwrap();
        });

        let (outcome, done, events) = run(
            &copy_entries(&archive, &[], &["a.txt", "b.txt"], &dest),
            [
                Reply::Password("wrong".to_owned()),
                Reply::Password("secret".to_owned()),
            ],
        );

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 2);
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
        let (outcome, done, events) = run(
            &copy_entries_with_password(&archive, &["a.txt"], &known, "secret"),
            [],
        );
        assert!(matches!(outcome, Outcome::Completed { .. }));
        assert_eq!(done, 1);
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, OperationEvent::NeedPassword { .. }))
        );

        let cancelled = dir.path().join("cancelled");
        fs::create_dir(&cancelled).unwrap();
        let (outcome, done, _) = run(
            &copy_entries(&archive, &[], &["a.txt"], &cancelled),
            [Reply::Cancel],
        );
        assert_eq!(outcome, Outcome::Cancelled);
        assert_eq!(done, 0);
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
            writer
                .add_file("a.txt", 3, None, None, &mut &b"new"[..])
                .unwrap();
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

        run_operation(&spec, None, &NestedArchives::default(), &replies, &report);

        assert!(matches!(
            events.into_inner().unwrap().last(),
            Some(OperationEvent::Finished(Outcome::Cancelled))
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
            writer
                .add_file("a.txt", 0, None, None, &mut io::empty())
                .unwrap();
        });

        let (outcome, _, _) = run(&copy_entries(&archive, &[], &["missing"], &dest), []);

        assert!(matches!(
            outcome,
            Outcome::Failed { path, error, .. }
                if path == archive.join("missing") && error.to_string() == "not found in the archive"
        ));
    }

    #[test]
    fn entries_of_a_nested_archive_are_copied_out() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let nested = archive_bytes(ArchiveFormat::TarGz, &[("d/f.txt", b"nested")]);
        write_archive(&archive, ArchiveFormat::Zip, &[("x/n.tar.gz", &nested)]);
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let mut spec = copy_entries(&archive, &["d"], &["f.txt"], &dest);
        if let Sources::Entries(entries) = &mut spec.sources {
            entries.path.outer = vec![OuterArchive {
                format: Some(ArchiveFormat::Zip),
                entry: vec!["x".to_owned(), "n.tar.gz".to_owned()],
            }];
            entries.path.format = Some(ArchiveFormat::TarGz);
        }

        let (outcome, done, _) = run(&spec, []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 1);
        assert_eq!(fs::read(dest.join("f.txt")).unwrap(), b"nested");
    }

    #[test]
    fn the_password_of_a_nested_archive_is_asked_for_under_its_path() {
        let dir = TempDir::new().unwrap();
        let inner = dir.path().join("inner.zip");
        write_zip(&inner, Some("secret"), |writer| {
            writer
                .add_file("a.txt", 1, None, None, &mut &b"a"[..])
                .unwrap();
        });
        let archive = dir.path().join("a.zip");
        write_archive(
            &archive,
            ArchiveFormat::Zip,
            &[("n.zip", &fs::read(&inner).unwrap())],
        );
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let mut spec = copy_entries(&archive, &[], &["a.txt"], &dest);
        if let Sources::Entries(entries) = &mut spec.sources {
            entries.path.outer = vec![OuterArchive {
                format: Some(ArchiveFormat::Zip),
                entry: vec!["n.zip".to_owned()],
            }];
        }

        let (outcome, _, events) = run(&spec, [Reply::Password("secret".to_owned())]);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert!(events.iter().any(|event| matches!(
            event,
            OperationEvent::NeedPassword { archive: asked, .. } if *asked == archive.join("n.zip")
        )));
    }

    #[test]
    fn copying_out_of_an_archive_behind_an_encrypted_entry_asks_for_its_password() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let nested = archive_bytes(ArchiveFormat::Zip, &[("f.txt", b"f")]);
        write_encrypted(
            &archive,
            ArchiveFormat::Zip,
            Some("pw"),
            &[("n.zip", &nested)],
        );
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let mut spec = copy_entries(&archive, &[], &["f.txt"], &dest);
        if let Sources::Entries(entries) = &mut spec.sources {
            entries.path.outer = vec![OuterArchive {
                format: Some(ArchiveFormat::Zip),
                entry: vec!["n.zip".to_owned()],
            }];
        }

        let (outcome, _, events) = run(
            &spec,
            [
                Reply::Password("bad".to_owned()),
                Reply::Password("pw".to_owned()),
            ],
        );

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(
            asked(&events),
            [(archive.clone(), 0, false), (archive.clone(), 0, true)]
        );
        assert_eq!(fs::read(dest.join("f.txt")).unwrap(), b"f");
    }

    #[test]
    fn extracting_an_encrypted_archive_behind_an_encrypted_entry_asks_only_for_its_own() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let nested = encrypted_bytes(ArchiveFormat::Zip, Some("inner"), &[("f.txt", b"f")]);
        write_encrypted(
            &archive,
            ArchiveFormat::Zip,
            Some("outer"),
            &[("n.zip", &nested)],
        );
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let entries = entries(&archive, &[], &["n.zip"], Some("outer"));

        let (outcome, _, events) = run(
            &extract_entries(entries, &dest, false),
            [Reply::Password("inner".to_owned())],
        );

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(asked(&events), [(archive.join("n.zip"), 1, false)]);
        assert_eq!(fs::read(dest.join("f.txt")).unwrap(), b"f");
    }

    #[test]
    fn a_cancel_while_a_nested_archive_is_copied_out_stops_the_copy_as_cancelled() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let nested = archive_bytes(ArchiveFormat::Zip, &[("f.txt", b"f")]);
        write_archive(&archive, ArchiveFormat::Zip, &[("n.zip", &nested)]);
        let mut entries = entries(&archive, &[], &["f.txt"], None);
        entries.path.outer = vec![OuterArchive {
            format: Some(ArchiveFormat::Zip),
            entry: vec!["n.zip".to_owned()],
        }];
        let (sender, replies) = mpsc::channel();
        sender.send(Reply::Cancel).unwrap();
        let store = NestedArchives::default();
        let worker = Worker {
            owner_window: None,
            nested: &store,
            replies: &replies,
            report: &|_| {},
            policy: None,
            done: 0,
            skipped: 0,
            last_progress: None,
        };

        let result = worker.open_entries(&entries, dir.path());

        assert!(matches!(result, Err(Stop::Cancelled)));
    }

    #[test]
    fn extract_unpacks_an_archive_inside_an_archive_into_a_directory_or_straight() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let nested = archive_bytes(ArchiveFormat::Zip, &[("f.txt", b"f")]);
        write_archive(&archive, ArchiveFormat::Zip, &[("x/n.zip", &nested)]);

        for into_dir in [true, false] {
            let dest = dir.path().join(format!("out-{into_dir}"));
            fs::create_dir(&dest).unwrap();
            let entries = entries(&archive, &["x"], &["n.zip"], None);

            let (outcome, done, _) = run(&extract_entries(entries, &dest, into_dir), []);

            assert_eq!(outcome, Outcome::Completed { skipped: 0 });
            assert_eq!(done, 1);
            let unpacked = if into_dir { dest.join("n") } else { dest };
            assert_eq!(fs::read(unpacked.join("f.txt")).unwrap(), b"f");
        }
    }

    #[test]
    fn extract_unpacks_an_archive_inside_a_nested_archive() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        let innermost = archive_bytes(ArchiveFormat::Tar, &[("f.txt", b"f")]);
        let nested = archive_bytes(ArchiveFormat::Zip, &[("m.tar", &innermost)]);
        write_archive(&archive, ArchiveFormat::Zip, &[("n.zip", &nested)]);
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        let mut entries = entries(&archive, &[], &["m.tar"], None);
        entries.path.outer = vec![OuterArchive {
            format: Some(ArchiveFormat::Zip),
            entry: vec!["n.zip".to_owned()],
        }];

        let (outcome, _, _) = run(&extract_entries(entries, &dest, true), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(fs::read(dest.join("m").join("f.txt")).unwrap(), b"f");
    }

    #[test]
    fn extracting_a_missing_entry_or_a_directory_inside_an_archive_fails() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        write_zip(&archive, None, |writer| {
            writer.add_dir("d", None, None).unwrap();
        });
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        for (name, message) in [
            ("missing.zip", "not found in the archive"),
            ("d", "not a regular file"),
        ] {
            let entries = entries(&archive, &[], &[name], None);

            let (outcome, _, _) = run(&extract_entries(entries, &dest, true), []);

            assert!(
                matches!(
                    &outcome,
                    Outcome::Failed { path, error }
                        if *path == archive.join(name) && error.to_string() == message
                ),
                "{outcome:?}"
            );
        }
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 0);
    }

    #[test]
    fn a_file_that_is_not_an_archive_fails_to_extract() {
        let dir = TempDir::new().unwrap();
        let text = dir.path().join("t.txt");
        fs::write(&text, b"text").unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        let (outcome, _, _) = run(&extract(std::slice::from_ref(&text), &dest), []);

        assert!(matches!(
            outcome,
            Outcome::Failed { path, error, .. }
                if path == text && error.to_string() == "not an archive"
        ));
    }

    #[test]
    fn a_tar_gz_with_a_foreign_extension_is_extracted_by_its_content() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(src.join("d")).unwrap();
        fs::write(src.join("d").join("f.txt"), b"hello").unwrap();
        run(
            &compress(&[src.join("d")], dir.path(), "x.tar.gz", None),
            [],
        );
        let archive = dir.path().join("x.bin");
        fs::rename(dir.path().join("x.tar.gz"), &archive).unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        let (outcome, _, _) = run(&extract(&[archive], &dest), []);

        assert!(matches!(outcome, Outcome::Completed { .. }));
        assert_eq!(fs::read(dest.join("d").join("f.txt")).unwrap(), b"hello");
    }

    #[test]
    fn a_gzip_that_holds_no_tar_is_not_a_tar_archive() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("t.bin");
        let mut encoder = flate2::write::GzEncoder::new(
            File::create(&archive).unwrap(),
            flate2::Compression::default(),
        );
        encoder.write_all(&b"just some text\n".repeat(64)).unwrap();
        encoder.finish().unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        let (outcome, _, _) = run(&extract(std::slice::from_ref(&archive), &dest), []);

        assert!(matches!(
            outcome,
            Outcome::Failed { path, error, .. }
                if path == archive && error.to_string() == "not a tar archive"
        ));
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

        let (outcome, done, events) = run(
            &compress(&[src.join("d"), src.join("top.txt")], &dest, "x.zip", None),
            [],
        );

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 5);
        assert_eq!(done_indices(&events), [0, 1]);
        let mut reader = open_archive(&dest.join("x.zip"), Some(ArchiveFormat::Zip)).unwrap();
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

        let (outcome, done, _) = run(&compress(&[src.join("link")], &dest, "x.zip", None), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(done, 1);
        let mut reader = open_archive(&dest.join("x.zip"), Some(ArchiveFormat::Zip)).unwrap();
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

        let (outcome, _, _) = run(
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
    fn compress_fails_at_an_output_name_that_leads_out_of_the_destination() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, b"a").unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        let (outcome, _, _) = run(
            &compress(std::slice::from_ref(&src), &dest, "../backup.zip", None),
            [],
        );

        assert!(matches!(outcome, Outcome::Failed { .. }), "{outcome:?}");
        assert!(!dir.path().join("backup.zip").exists());
    }

    #[test]
    fn compress_with_a_password_encrypts_every_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("a.txt");
        fs::write(&src, b"a").unwrap();
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();

        let (outcome, done, _) = run(&compress(&[src], &dest, "a.zip", Some("pw")), []);

        assert!(matches!(outcome, Outcome::Completed { .. }));
        assert_eq!(done, 1);
        let mut reader = open_archive(&dest.join("a.zip"), Some(ArchiveFormat::Zip)).unwrap();
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

        let (outcome, done, _) = run(
            &compress(std::slice::from_ref(&src), &dest, "a.zip", None),
            [Reply::Skip { all: false }],
        );
        assert_eq!(outcome, Outcome::Completed { skipped: 1 });
        assert_eq!(done, 0);
        assert_eq!(fs::read(dest.join("a.zip")).unwrap(), b"old");

        let (outcome, done, _) = run(
            &compress(std::slice::from_ref(&src), &dest, "a.zip", None),
            [Reply::Overwrite { all: false }],
        );
        assert!(matches!(outcome, Outcome::Completed { .. }));
        assert_eq!(done, 1);
        assert!(open_archive(&dest.join("a.zip"), Some(ArchiveFormat::Zip)).is_ok());

        let (outcome, _, _) = run(
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

        let (outcome, _, _) = run(
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
    fn a_compression_renamed_onto_one_of_its_sources_keeps_that_source() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("a.txt"), b"a").unwrap();
        fs::write(src.join("b.zip"), b"the original b.zip").unwrap();
        fs::write(src.join("x.zip"), b"old").unwrap();

        let (outcome, _, _) = run(
            &compress(&[src.join("a.txt"), src.join("b.zip")], &src, "x.zip", None),
            [
                Reply::Rename("b.zip".into()),
                Reply::Overwrite { all: false },
            ],
        );

        assert!(
            matches!(
                &outcome,
                Outcome::Failed { error, .. } if error.to_string() == "the output is inside the source"
            ),
            "outcome: {outcome:?}"
        );
        assert_eq!(
            fs::read(src.join("b.zip")).unwrap(),
            b"the original b.zip",
            "outcome: {outcome:?}"
        );
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn a_compression_onto_a_source_spelled_in_another_case_keeps_that_source() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("Foo");
        fs::create_dir(&src).unwrap();
        fs::write(src.join("a.txt"), b"a").unwrap();
        fs::write(src.join("foo.zip"), b"the original foo.zip").unwrap();

        let (outcome, _, events) = run(
            &compress(
                &[src.join("a.txt"), src.join("foo.zip")],
                &src,
                "Foo.zip",
                None,
            ),
            [Reply::Overwrite { all: false }],
        );

        assert!(
            matches!(
                &outcome,
                Outcome::Failed { error, .. } if error.to_string() == "the output is inside the source"
            ),
            "outcome: {outcome:?}"
        );
        assert_eq!(
            fs::read(src.join("foo.zip")).unwrap(),
            b"the original foo.zip",
            "outcome: {outcome:?}, events: {events:?}"
        );
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
            &NestedArchives::default(),
            &replies,
            &|event| {
                events.lock().unwrap().push(event);
            },
        );

        assert!(matches!(
            events.lock().unwrap().last(),
            Some(OperationEvent::Finished(Outcome::Cancelled))
        ));
        assert!(!dest.join("a.zip").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_compression_keeps_the_archive_it_was_to_replace() {
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
        fs::write(dest.join("x.zip"), b"old").unwrap();

        let (outcome, _, _) = run(
            &compress(std::slice::from_ref(&src), &dest, "x.zip", None),
            [Reply::Overwrite { all: false }],
        );

        assert!(matches!(outcome, Outcome::Failed { .. }), "{outcome:?}");
        assert_eq!(fs::read(dest.join("x.zip")).unwrap(), b"old");
        assert_eq!(fs::read_dir(&dest).unwrap().count(), 1);
    }

    #[test]
    fn a_target_is_a_directory_only_when_no_link_is_on_the_way_to_it() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join("d/sub")).unwrap();
        fs::write(dir.path().join("f"), b"").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("d", dir.path().join("l")).unwrap();
        // Making a link needs a privilege on Windows.
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir("d", dir.path().join("l")).is_err() {
            return;
        }

        for target in ["d", "d/sub", "./d/../d", ".."] {
            assert!(links_to_dir(dir.path(), target), "{target}");
        }
        for target in ["f", "missing", "l", "l/sub", "/"] {
            assert!(!links_to_dir(dir.path(), target), "{target}");
        }
    }

    #[test]
    fn only_a_target_without_a_root_or_a_drive_is_plain_relative() {
        for target in [
            "//host/share",
            r"\\host\share",
            "C:/x",
            "/x",
            "C:x",
            "sub/C:/x",
        ] {
            assert!(!is_plain_relative(target), "{target}");
        }
        for target in ["sub/f.txt", "../d"] {
            assert!(is_plain_relative(target), "{target}");
        }
    }

    #[test]
    fn extract_to_temp_makes_a_fresh_directory_and_reports_it() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        write_zip(&archive, None, |writer| {
            writer
                .add_file("a.txt", 1, None, None, &mut &b"a"[..])
                .unwrap();
        });
        let spec = OperationSpec {
            kind: OperationKind::ExtractToTemp,
            sources: Sources::Entries(entries(&archive, &[], &["a.txt"], None)),
            dest: None,
            output: None,
        };

        let (outcome, done, events) = run(&spec, []);

        assert!(matches!(outcome, Outcome::Completed { .. }));
        assert_eq!(done, 1);
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

    #[cfg(target_os = "macos")]
    #[test]
    fn quarantine_is_carried_to_extracted_files() {
        use std::process::Command;

        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("download.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, None, |writer| {
            writer
                .add_file(
                    "run.command",
                    10,
                    None,
                    Some(0o444),
                    &mut &b"#!/bin/sh\n"[..],
                )
                .unwrap();
        });
        let set = Command::new("/usr/bin/xattr")
            .args(["-w", "com.apple.quarantine", "0083;65a00000;Safari;"])
            .arg(&archive)
            .status()
            .unwrap();
        assert!(set.success());
        let has = |path: &Path| {
            Command::new("/usr/bin/xattr")
                .args(["-p", "com.apple.quarantine"])
                .arg(path)
                .output()
                .unwrap()
                .status
                .success()
        };

        let (outcome, _, _) = run(&extract(std::slice::from_ref(&archive), &dest), []);
        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        let spec = OperationSpec {
            kind: OperationKind::ExtractToTemp,
            sources: Sources::Entries(entries(&archive, &[], &["run.command"], None)),
            dest: None,
            output: None,
        };
        let (_, _, events) = run(&spec, []);
        let Some(OperationEvent::TempDir(temp)) = events.first() else {
            panic!();
        };
        let extracted = has(&dest.join("run.command"));
        let in_temp = has(&temp.join("run.command"));
        fs::remove_dir_all(temp).unwrap();

        assert!(extracted, "the extracted file is not quarantined");
        assert!(in_temp, "the file extracted to open is not quarantined");
    }

    #[cfg(windows)]
    #[test]
    fn the_zone_of_the_archive_is_carried_to_extracted_files() {
        const ZONE: &[u8] = b"[ZoneTransfer]\r\nZoneId=3\r\n";

        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("download.zip");
        let dest = dir.path().join("out");
        fs::create_dir(&dest).unwrap();
        write_zip(&archive, None, |writer| {
            writer
                .add_file("a.txt", 1, None, None, &mut &b"a"[..])
                .unwrap();
        });
        if fs::write(zone_identifier(&archive), ZONE).is_err() {
            return;
        }

        let (outcome, _, _) = run(&extract(std::slice::from_ref(&archive), &dest), []);

        assert_eq!(outcome, Outcome::Completed { skipped: 0 });
        assert_eq!(
            fs::read(zone_identifier(&dest.join("a.txt"))).unwrap(),
            ZONE
        );
    }

    fn run(
        spec: &OperationSpec,
        script: impl IntoIterator<Item = Reply>,
    ) -> (Outcome, usize, Vec<OperationEvent>) {
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
        let nested = NestedArchives::default();
        let mut worker = Worker {
            owner_window: None,
            nested: &nested,
            replies: &replies,
            report: &report,
            policy: None,
            done: 0,
            skipped: 0,
            last_progress: None,
        };
        let outcome = worker.run(spec);
        let done = worker.done;
        (outcome, done, events.into_inner().unwrap())
    }

    // The archive, level, and retry of each password asked for.
    fn asked(events: &[OperationEvent]) -> Vec<(PathBuf, usize, bool)> {
        events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::NeedPassword {
                    archive,
                    level,
                    retry,
                } => Some((archive.clone(), *level, *retry)),
                _ => None,
            })
            .collect()
    }

    fn passwords(levels: &[Option<&str>]) -> Passwords {
        let mut passwords = Passwords::default();
        for (level, password) in levels.iter().enumerate() {
            passwords.set(level, password.map(ToOwned::to_owned));
        }
        passwords
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

    fn skipped_errors(events: &[OperationEvent]) -> Vec<String> {
        events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::Skipped { error, .. } => Some(error.to_string()),
                _ => None,
            })
            .collect()
    }

    fn write_zip(path: &Path, password: Option<&str>, fill: impl FnOnce(&mut ZipArchiveWriter)) {
        let mut writer = ZipArchiveWriter::new(File::create(path).unwrap(), password);
        fill(&mut writer);
        Box::new(writer).finish().unwrap();
    }

    // A zip whose first entry `l` is a link with `target` as its content,
    // which `add_symlink` takes only as a string, followed by `f.txt`. The
    // link is written as a file and its type is set in the central directory.
    #[cfg(unix)]
    fn write_zip_with_link(path: &Path, target: &mut dyn io::Read) {
        const LINK: u32 = 0o120_777;
        let mut writer = ::zip::ZipWriter::new(File::create(path).unwrap());
        let options = ::zip::write::SimpleFileOptions::default()
            .compression_method(::zip::CompressionMethod::Deflated)
            .unix_permissions(0o777);
        writer.start_file("l", options).unwrap();
        io::copy(target, &mut writer).unwrap();
        writer.start_file("f.txt", options).unwrap();
        writer.write_all(b"f").unwrap();
        writer.finish().unwrap();
        let mut bytes = fs::read(path).unwrap();
        let central = bytes
            .windows(4)
            .rposition(|window| window == b"PK\x01\x02")
            .unwrap();
        // The last central header is that of `f.txt`; the one before it, 46
        // bytes and the one-byte name earlier, is that of `l`.
        let central = central - 47;
        assert_eq!(&bytes[central..central + 4], b"PK\x01\x02");
        bytes[central + 38..central + 42].copy_from_slice(&(LINK << 16).to_le_bytes());
        fs::write(path, bytes).unwrap();
    }

    // Names and link targets are written as given, which the crate's own
    // setters would normalize or refuse.
    fn write_tar(path: &Path, members: &[(&str, EntryType, &str, &[u8])]) {
        let mut builder = tar::Builder::new(File::create(path).unwrap());
        for (name, kind, link, data) in members {
            let mut header = Header::new_gnu();
            header.set_entry_type(*kind);
            header.set_size(data.len() as u64);
            header.set_mode(if *kind == EntryType::Directory || name.ends_with('/') {
                0o755
            } else {
                0o644
            });
            header.as_old_mut().name[..name.len()].copy_from_slice(name.as_bytes());
            header.as_old_mut().linkname[..link.len()].copy_from_slice(link.as_bytes());
            header.set_cksum();
            builder.append(&header, *data).unwrap();
        }
        builder.into_inner().unwrap().flush().unwrap();
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
                outer: Vec::new(),
                format: ArchiveFormat::from_name(archive.file_name().unwrap()),
                inner: inner.iter().map(|s| (*s).to_owned()).collect(),
            },
            names: names.iter().map(|s| (*s).to_owned()).collect(),
            passwords: passwords(&[password]),
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

    fn extract_entries(entries: ArchiveEntries, dest: &Path, into_dir: bool) -> OperationSpec {
        OperationSpec {
            kind: OperationKind::Extract { into_dir },
            sources: Sources::Entries(entries),
            dest: Some(dest.to_path_buf()),
            output: None,
        }
    }

    fn extract(archives: &[PathBuf], dest: &Path) -> OperationSpec {
        OperationSpec {
            kind: OperationKind::Extract { into_dir: false },
            sources: Sources::Paths(archives.to_vec()),
            dest: Some(dest.to_path_buf()),
            output: None,
        }
    }

    fn extract_into_dirs(archives: &[PathBuf], dest: &Path) -> OperationSpec {
        OperationSpec {
            kind: OperationKind::Extract { into_dir: true },
            ..extract(archives, dest)
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
                format: ArchiveFormat::from_name(name.as_ref()).unwrap(),
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
