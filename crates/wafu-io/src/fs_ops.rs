use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io;
use std::path::Path;

/// Renames within a directory. An existing name is an error unless it is the
/// same file under another spelling of its case.
pub fn rename(from: &Path, to: &Path) -> io::Result<()> {
    if fs::symlink_metadata(to).is_ok()
        && !(differ_only_in_case(from, to) && same_file::is_same_file(from, to).unwrap_or(false))
    {
        return Err(already_exists());
    }
    fs::rename(from, to)
}

/// Creates a directory and any missing parents. An existing entry of any
/// kind is an error.
pub fn mkdir(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok() {
        return Err(already_exists());
    }
    fs::create_dir_all(path)
}

/// Creates an empty file. An existing entry is an error.
pub fn create_file(path: &Path) -> io::Result<()> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(drop)
}

/// Opens the path with whatever the OS associates with it, without waiting.
pub fn open_with_os(path: &Path) -> io::Result<()> {
    open::that_detached(path)
}

/// Puts text on the clipboard. The clipboard is created on first use and
/// kept, since on X11 the text is only served while it exists.
pub fn set_clipboard(
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

fn differ_only_in_case(a: &Path, b: &Path) -> bool {
    a.parent() == b.parent()
        && match (
            a.file_name().and_then(OsStr::to_str),
            b.file_name().and_then(OsStr::to_str),
        ) {
            (Some(a), Some(b)) => a.to_lowercase() == b.to_lowercase(),
            _ => false,
        }
}

fn already_exists() -> io::Error {
    io::Error::new(io::ErrorKind::AlreadyExists, "already exists")
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use tempfile::TempDir;

    use super::*;

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
    fn rename_to_a_new_name_works() {
        let dir = TempDir::new().unwrap();
        File::create(dir.path().join("a")).unwrap();

        rename(&dir.path().join("a"), &dir.path().join("b")).unwrap();

        assert!(!dir.path().join("a").exists());
        assert!(dir.path().join("b").exists());
    }

    // On a case-insensitive file system the new name already "exists" as the
    // same file; on a case-sensitive one it is simply free. Both must work.
    #[test]
    fn rename_changing_only_the_case_works() {
        let dir = TempDir::new().unwrap();
        File::create(dir.path().join("a.txt")).unwrap();

        rename(&dir.path().join("a.txt"), &dir.path().join("A.txt")).unwrap();

        let names: Vec<String> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["A.txt"]);
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

    #[cfg(unix)]
    #[test]
    fn rename_refuses_a_target_onto_its_link() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("target"), "data").unwrap();
        std::os::unix::fs::symlink("target", dir.path().join("link")).unwrap();

        let error = rename(&dir.path().join("target"), &dir.path().join("link")).unwrap_err();

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
        assert_eq!(
            create_file(&path).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
    }
}
