use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;

use wafu_core::nfc;

pub(crate) fn rename(from: &Path, to: &Path) -> io::Result<()> {
    if is_taken(from, to) {
        return Err(already_exists());
    }
    fs::rename(from, to)
}

pub(crate) fn is_taken(from: &Path, to: &Path) -> bool {
    fs::symlink_metadata(to).is_ok()
        && !(differ_only_in_case(from, to)
            && is_same_file(from, to).unwrap_or(false)
            && is_alone(from, to))
}

// Opening the files for reading, as `same_file::is_same_file` does, fails on
// a file that cannot be read and never returns on a FIFO; neither may pass
// for "another file".
#[cfg(unix)]
pub(crate) fn is_same_file(a: &Path, b: &Path) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let id = |meta: fs::Metadata| (meta.dev(), meta.ino());
    let entries = id(fs::symlink_metadata(a)?) == id(fs::symlink_metadata(b)?);
    // Following the links fails for a broken one, which the entries
    // themselves still tell apart.
    Ok(entries
        || fs::metadata(a)
            .and_then(|a| Ok(id(a) == id(fs::metadata(b)?)))
            .unwrap_or(false))
}

// Access mode 0 asks for no rights to the file, so one that cannot be read can
// still be compared. Backup semantics is needed to open a directory, which the
// compress output check may pass as its source.
#[cfg(windows)]
pub(crate) fn is_same_file(a: &Path, b: &Path) -> io::Result<bool> {
    use std::os::windows::fs::OpenOptionsExt;

    use windows::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    };

    let handle = |path: &Path, flags: u32| {
        let file = OpenOptions::new()
            .access_mode(0)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS.0 | flags)
            .open(path)?;
        same_file::Handle::from_file(file)
    };
    let entry = |path| handle(path, FILE_FLAG_OPEN_REPARSE_POINT.0);
    let entries = entry(a)? == entry(b)?;
    // Following the links fails for a broken one, which the entries
    // themselves still tell apart.
    Ok(entries
        || handle(a, 0)
            .and_then(|a| Ok(a == handle(b, 0)?))
            .unwrap_or(false))
}

pub(crate) fn mkdir(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        return Err(already_exists());
    }
    fs::create_dir_all(path)
}

pub(crate) fn create_file(path: &Path) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(drop)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                already_exists()
            } else {
                error
            }
        })
}

pub(crate) fn open_with_os(path: &Path) -> io::Result<()> {
    open::that_detached(path)
}

// The clipboard is created on first use and kept, since on X11 the text is
// only served while it exists.
pub(crate) fn set_clipboard(
    clipboard: &mut Option<arboard::Clipboard>,
    text: &str,
) -> Result<(), arboard::Error> {
    if clipboard.is_none() {
        *clipboard = Some(arboard::Clipboard::new()?);
    }
    let result = clipboard
        .as_mut()
        .expect("the clipboard was just created")
        .set_text(text);
    if result.is_err() {
        // A broken clipboard is made again next time.
        *clipboard = None;
    }
    result
}

pub(crate) fn already_exists() -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, "already exists")
}

fn differ_only_in_case(a: &Path, b: &Path) -> bool {
    a.parent() == b.parent()
        && match (
            a.file_name().and_then(OsStr::to_str),
            b.file_name().and_then(OsStr::to_str),
        ) {
            // A typed name is composed, while the one on disk may not be.
            (Some(a), Some(b)) => nfc(a).to_lowercase() == nfc(b).to_lowercase(),
            _ => false,
        }
}

// Being the same file does not make `to` the entry `from`: a link and its
// target, or two hard links, are the same file under separate entries, and
// renaming one onto the other loses an entry.
fn is_alone(from: &Path, to: &Path) -> bool {
    let (Some(dir), Some(from_name), Some(to_name)) = (
        to.parent(),
        from.file_name(),
        to.file_name().and_then(OsStr::to_str),
    ) else {
        return false;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    let to_name = nfc(to_name);
    // A file system that ignores normalization but not case finds a decomposed
    // entry under the composed name.
    !entries.flatten().any(|entry| {
        let name = entry.file_name();
        name != from_name && name.to_str().is_some_and(|name| nfc(name) == to_name)
    })
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use tempfile::TempDir;

    use super::*;
    use crate::watch::tests::symlink;

    #[test]
    fn rename_refuses_an_existing_name() {
        let dir = TempDir::new().unwrap();
        File::create(dir.path().join("a")).unwrap();
        File::create(dir.path().join("b")).unwrap();

        let error = rename(&dir.path().join("a"), &dir.path().join("b")).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(error.to_string(), "already exists");
        assert!(dir.path().join("a").exists());
    }

    #[test]
    fn rename_refuses_a_link_onto_its_target_named_in_another_case() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("A"), "data").unwrap();
        if dir.path().join("a").exists() || !symlink(Path::new("A"), &dir.path().join("a")) {
            return;
        }

        let error = rename(&dir.path().join("a"), &dir.path().join("A")).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn rename_refuses_a_hard_link_named_in_another_case() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("A"), "data").unwrap();
        if fs::hard_link(dir.path().join("A"), dir.path().join("a")).is_err() {
            return;
        }

        let error = rename(&dir.path().join("a"), &dir.path().join("A")).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn differ_only_in_case_ignores_normalization() {
        let dir = Path::new("d");
        assert!(differ_only_in_case(
            &dir.join("\u{304b}\u{3099}a.txt"),
            &dir.join("\u{304c}A.txt")
        ));
    }

    #[cfg(unix)]
    #[test]
    fn rename_refuses_a_link_onto_its_target() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("target"), "data").unwrap();
        std::os::unix::fs::symlink("target", dir.path().join("link")).unwrap();

        let error = rename(&dir.path().join("link"), &dir.path().join("target")).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            fs::symlink_metadata(dir.path().join("link"))
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("target")).unwrap(),
            "data"
        );
    }

    #[test]
    fn rename_refuses_another_hard_link_to_the_same_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a"), "data").unwrap();
        fs::hard_link(dir.path().join("a"), dir.path().join("b")).unwrap();

        let error = rename(&dir.path().join("a"), &dir.path().join("b")).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(dir.path().join("a").exists());
        assert!(dir.path().join("b").exists());
    }

    #[test]
    fn a_link_is_the_same_file_as_its_target() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("target"), "data").unwrap();
        if !symlink(Path::new("target"), &dir.path().join("link")) {
            return;
        }

        assert!(is_same_file(&dir.path().join("link"), &dir.path().join("target")).unwrap());
    }

    #[test]
    fn an_unreadable_file_is_the_same_file_as_itself() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a");
        fs::write(&file, "data").unwrap();
        deny_reading(&file);
        // A user who can read it anyway (root) has nothing to check.
        if File::open(&file).is_ok() {
            return;
        }

        assert!(is_same_file(&file, &file).unwrap());
    }

    #[cfg(unix)]
    #[test]
    fn a_fifo_is_the_same_file_as_itself_without_blocking() {
        use std::sync::mpsc;
        use std::time::Duration;

        let dir = TempDir::new().unwrap();
        let fifo = dir.path().join("p");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());
        let (sender, results) = mpsc::channel();
        let path = fifo.clone();
        std::thread::spawn(move || {
            let _ = sender.send(is_same_file(&path, &path));
        });

        let result = results.recv_timeout(Duration::from_secs(3)).ok();
        if result.is_none() {
            // Releases the thread stuck opening the FIFO for reading.
            std::thread::spawn(move || OpenOptions::new().write(true).open(fifo));
            let _ = results.recv_timeout(Duration::from_secs(3));
        }

        assert!(
            matches!(result, Some(Ok(true))),
            "blocked or failed: {result:?}"
        );
    }

    #[test]
    fn mkdir_creates_parents_and_refuses_existing_entries() {
        let dir = TempDir::new().unwrap();

        mkdir(&dir.path().join("a").join("b").join("c")).unwrap();
        assert!(dir.path().join("a").join("b").join("c").is_dir());

        let error = mkdir(&dir.path().join("a")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);

        File::create(dir.path().join("f")).unwrap();
        assert!(mkdir(&dir.path().join("f")).is_err());
    }

    #[test]
    fn create_file_makes_an_empty_file_once() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("n.txt");

        create_file(&path).unwrap();

        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        let error = create_file(&path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(error.to_string(), "already exists");
    }

    #[cfg(unix)]
    fn deny_reading(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o000)).unwrap();
    }

    #[cfg(windows)]
    fn deny_reading(path: &Path) {
        // `S-1-1-0` is Everyone, under whatever name the system gives it.
        let status = std::process::Command::new("icacls")
            .arg(path)
            .args(["/deny", "*S-1-1-0:(RD)"])
            .status()
            .unwrap();
        assert!(status.success());
    }
}
