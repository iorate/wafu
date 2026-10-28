use std::ffi::OsString;
use std::fs::{self, Metadata};
use std::io;
use std::path::Path;

const CANCEL_CHECK_INTERVAL: usize = 256;

use wafu_core::{Directory, Entry, EntryKind, Permissions, SpecialKind, TargetKind};

use crate::time::timestamp;

/// Lists a directory in the order the file system returns it, along with the
/// directory itself.
///
/// An entry whose metadata cannot be read is kept as a plain file with no size
/// or time, so that it still shows up in the listing.
pub fn read_dir(path: &Path) -> io::Result<Directory> {
    read_dir_with(path, || false)
}

/// Like [`read_dir`], but gives up with [`io::ErrorKind::Interrupted`] once
/// `cancelled` returns `true`. It is asked every few hundred entries, so a
/// large directory can be abandoned part way through.
pub fn read_dir_with(path: &Path, cancelled: impl Fn() -> bool) -> io::Result<Directory> {
    let mut entries = Vec::new();
    for (index, dir_entry) in fs::read_dir(path)?.enumerate() {
        if index % CANCEL_CHECK_INTERVAL == 0 && cancelled() {
            return Err(io::Error::from(io::ErrorKind::Interrupted));
        }
        let dir_entry = dir_entry?;
        let name = dir_entry.file_name();
        let metadata = dir_entry.metadata().ok();
        entries.push(entry(&dir_entry.path(), name, metadata.as_ref()));
    }
    let own = fs::metadata(path)
        .ok()
        .map(|metadata| own_entry(path, &metadata));
    Ok(Directory { own, entries })
}

/// The item at `path` itself, from metadata that followed links.
pub(crate) fn own_entry(path: &Path, metadata: &Metadata) -> Entry {
    let name = path
        .file_name()
        .map_or_else(OsString::new, ToOwned::to_owned);
    entry(path, name, Some(metadata))
}

fn entry(path: &Path, name: OsString, metadata: Option<&Metadata>) -> Entry {
    let hidden = is_hidden(&name, metadata);
    let Some(metadata) = metadata else {
        return Entry {
            name,
            kind: EntryKind::File { executable: false },
            size: 0,
            modified: None,
            hidden,
            permissions: None,
            link_target: None,
        };
    };
    let file_type = metadata.file_type();
    let link_target = file_type
        .is_symlink()
        .then(|| fs::read_link(path).ok())
        .flatten()
        .map(std::path::PathBuf::into_os_string);
    // Size and time follow the link target; a broken link has neither.
    let (kind, details) = if file_type.is_symlink() {
        match fs::metadata(path) {
            Ok(target) => {
                let kind = if target.is_dir() {
                    TargetKind::Dir
                } else {
                    TargetKind::File
                };
                (EntryKind::Symlink { target: Some(kind) }, Some(target))
            }
            Err(_) => (EntryKind::Symlink { target: None }, None),
        }
    } else if file_type.is_dir() {
        (EntryKind::Dir, Some(metadata.clone()))
    } else if let Some(special) = special_kind(file_type) {
        (EntryKind::Special(special), Some(metadata.clone()))
    } else {
        let executable = is_executable(&name, metadata);
        (EntryKind::File { executable }, Some(metadata.clone()))
    };
    Entry {
        name,
        kind,
        size: details.as_ref().map_or(0, Metadata::len),
        modified: details
            .and_then(|details| details.modified().ok())
            .map(timestamp),
        hidden,
        permissions: Some(permissions(metadata)),
        link_target,
    }
}

#[cfg(windows)]
fn permissions(metadata: &Metadata) -> Permissions {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_READONLY: u32 = 0x1;

    Permissions::Attributes {
        readonly: metadata.file_attributes() & FILE_ATTRIBUTE_READONLY != 0,
    }
}

#[cfg(not(windows))]
fn permissions(metadata: &Metadata) -> Permissions {
    use std::os::unix::fs::PermissionsExt;

    Permissions::Mode(metadata.permissions().mode() & 0o7777)
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

#[cfg(unix)]
fn special_kind(file_type: fs::FileType) -> Option<SpecialKind> {
    use std::os::unix::fs::FileTypeExt;

    if file_type.is_fifo() {
        Some(SpecialKind::Fifo)
    } else if file_type.is_socket() {
        Some(SpecialKind::Socket)
    } else if file_type.is_block_device() {
        Some(SpecialKind::BlockDevice)
    } else if file_type.is_char_device() {
        Some(SpecialKind::CharDevice)
    } else {
        None
    }
}

#[cfg(not(unix))]
fn special_kind(_file_type: fs::FileType) -> Option<SpecialKind> {
    None
}

#[cfg(unix)]
fn is_executable(_name: &OsString, metadata: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;

    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_executable(name: &OsString, _metadata: &Metadata) -> bool {
    Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
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

        let entries = read_dir(dir.path()).unwrap().entries;

        let file = find(&entries, "a.txt");
        assert_eq!(file.kind, EntryKind::File { executable: false });
        assert_eq!(file.size, 5);
        assert!(file.modified.is_some());
        assert_eq!(find(&entries, "sub").kind, EntryKind::Dir);
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn the_directory_itself_is_reported() {
        let dir = TempDir::new().unwrap();

        let own = read_dir(dir.path()).unwrap().own.unwrap();

        assert_eq!(Some(own.name.as_os_str()), dir.path().file_name());
        assert_eq!(own.kind, EntryKind::Dir);
        assert!(own.modified.is_some());
        assert!(own.permissions.is_some());
    }

    #[test]
    fn cancelled_read_is_interrupted() {
        let dir = TempDir::new().unwrap();
        fs::File::create(dir.path().join("a")).unwrap();

        let error = read_dir_with(dir.path(), || true).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Interrupted);
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

        let entries = read_dir(dir.path()).unwrap().entries;

        assert!(find(&entries, ".hidden").hidden);
        assert!(!find(&entries, "shown").hidden);
    }

    #[cfg(unix)]
    #[test]
    fn executable_bits_and_fifos_are_reported() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("run"), b"#!/bin/sh\n").unwrap();
        fs::set_permissions(dir.path().join("run"), fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(dir.path().join("plain"), b"").unwrap();
        let status = std::process::Command::new("mkfifo")
            .arg(dir.path().join("fifo"))
            .status()
            .unwrap();
        assert!(status.success());

        let entries = read_dir(dir.path()).unwrap().entries;

        let run = find(&entries, "run");
        assert_eq!(run.kind, EntryKind::File { executable: true });
        assert_eq!(run.permissions, Some(Permissions::Mode(0o755)));
        assert_eq!(run.link_target, None);
        assert_eq!(
            find(&entries, "plain").kind,
            EntryKind::File { executable: false }
        );
        let fifo = find(&entries, "fifo");
        assert_eq!(fifo.kind, EntryKind::Special(SpecialKind::Fifo));
        assert!(fifo.modified.is_some());
    }

    #[cfg(windows)]
    #[test]
    fn exe_files_are_executable() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a.exe"), b"").unwrap();
        fs::write(dir.path().join("B.EXE"), b"").unwrap();
        fs::write(dir.path().join("c.txt"), b"").unwrap();

        let entries = read_dir(dir.path()).unwrap().entries;

        assert_eq!(
            find(&entries, "a.exe").kind,
            EntryKind::File { executable: true }
        );
        assert_eq!(
            find(&entries, "B.EXE").kind,
            EntryKind::File { executable: true }
        );
        assert_eq!(
            find(&entries, "c.txt").kind,
            EntryKind::File { executable: false }
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_report_their_target_kind() {
        use std::ffi::OsStr;
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("file"), b"hello").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        symlink("file", dir.path().join("to_file")).unwrap();
        symlink("sub", dir.path().join("to_dir")).unwrap();
        symlink("missing", dir.path().join("broken")).unwrap();

        let entries = read_dir(dir.path()).unwrap().entries;

        let to_file = find(&entries, "to_file");
        assert_eq!(
            to_file.kind,
            EntryKind::Symlink {
                target: Some(TargetKind::File)
            }
        );
        assert_eq!(to_file.size, 5);
        assert_eq!(to_file.modified, find(&entries, "file").modified);
        assert_eq!(to_file.link_target.as_deref(), Some(OsStr::new("file")));
        assert!(matches!(to_file.permissions, Some(Permissions::Mode(_))));
        assert_eq!(
            find(&entries, "to_dir").kind,
            EntryKind::Symlink {
                target: Some(TargetKind::Dir)
            }
        );
        let broken = find(&entries, "broken");
        assert_eq!(broken.kind, EntryKind::Symlink { target: None });
        assert_eq!(broken.size, 0);
        assert_eq!(broken.modified, None);
        assert_eq!(broken.link_target.as_deref(), Some(OsStr::new("missing")));
    }

    #[cfg(windows)]
    #[test]
    fn readonly_attribute_is_reported() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("locked.txt"), b"").unwrap();
        let mut permissions = fs::metadata(dir.path().join("locked.txt"))
            .unwrap()
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(dir.path().join("locked.txt"), permissions).unwrap();
        fs::write(dir.path().join("open.txt"), b"").unwrap();

        let entries = read_dir(dir.path()).unwrap().entries;

        assert_eq!(
            find(&entries, "locked.txt").permissions,
            Some(Permissions::Attributes { readonly: true })
        );
        assert_eq!(
            find(&entries, "open.txt").permissions,
            Some(Permissions::Attributes { readonly: false })
        );
    }

    fn find<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("{name} should be listed"))
    }
}
