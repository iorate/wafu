use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io;
use std::path::Path;

use wafu_core::{Entry, EntryKind, TargetKind};

use crate::time::timestamp;

/// Lists a directory in the order the file system returns it.
///
/// An entry whose metadata cannot be read is kept as a plain file with no size
/// or time, so that it still shows up in the listing.
pub fn read_dir(path: &Path) -> io::Result<Vec<Entry>> {
    let mut entries = Vec::new();
    for dir_entry in fs::read_dir(path)? {
        let dir_entry = dir_entry?;
        let name = dir_entry.file_name();
        let metadata = dir_entry.metadata().ok();
        entries.push(entry(&dir_entry.path(), name, metadata.as_ref()));
    }
    Ok(entries)
}

fn entry(path: &Path, name: OsString, metadata: Option<&Metadata>) -> Entry {
    let kind = metadata.map_or(EntryKind::File, |metadata| {
        let file_type = metadata.file_type();
        if file_type.is_symlink() {
            let target = fs::metadata(path).ok().map(|target| {
                if target.is_dir() {
                    TargetKind::Dir
                } else {
                    TargetKind::File
                }
            });
            EntryKind::Symlink { target }
        } else if file_type.is_dir() {
            EntryKind::Dir
        } else {
            EntryKind::File
        }
    });
    Entry {
        hidden: is_hidden(&name, metadata),
        size: metadata.map_or(0, Metadata::len),
        modified: metadata
            .and_then(|metadata| metadata.modified().ok())
            .map(timestamp),
        name,
        kind,
    }
}

#[cfg(windows)]
fn is_hidden(_name: &OsString, metadata: Option<&Metadata>) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

    metadata.is_some_and(|metadata| metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0)
}

#[cfg(not(windows))]
fn is_hidden(name: &OsString, _metadata: Option<&Metadata>) -> bool {
    name.as_encoded_bytes().first() == Some(&b'.')
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn lists_files_and_directories_with_size() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.txt"), b"hello").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();

        let entries = read_dir(dir.path()).unwrap();

        let file = find(&entries, "a.txt");
        assert_eq!(file.kind, EntryKind::File);
        assert_eq!(file.size, 5);
        assert!(file.modified.is_some());
        assert_eq!(find(&entries, "sub").kind, EntryKind::Dir);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn missing_directory_is_an_error() {
        let dir = TempDir::new().unwrap();

        assert!(read_dir(&dir.path().join("missing")).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn dot_files_are_hidden() {
        let dir = TempDir::new().unwrap();
        fs::File::create(dir.path().join(".hidden")).unwrap();
        fs::File::create(dir.path().join("shown")).unwrap();

        let entries = read_dir(dir.path()).unwrap();

        assert!(find(&entries, ".hidden").hidden);
        assert!(!find(&entries, "shown").hidden);
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_report_their_target_kind() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        fs::File::create(dir.path().join("file")).unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        symlink("file", dir.path().join("to_file")).unwrap();
        symlink("sub", dir.path().join("to_dir")).unwrap();
        symlink("missing", dir.path().join("broken")).unwrap();

        let entries = read_dir(dir.path()).unwrap();

        assert_eq!(
            find(&entries, "to_file").kind,
            EntryKind::Symlink {
                target: Some(TargetKind::File)
            }
        );
        assert_eq!(
            find(&entries, "to_dir").kind,
            EntryKind::Symlink {
                target: Some(TargetKind::Dir)
            }
        );
        assert_eq!(
            find(&entries, "broken").kind,
            EntryKind::Symlink { target: None }
        );
    }

    fn find<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("{name} should be listed"))
    }
}
