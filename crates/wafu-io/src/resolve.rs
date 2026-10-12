use std::ffi::OsString;
use std::fs;
use std::path::Path;

use wafu_core::Resolved;

/// Finds the directory to show for a path that may not exist: the path itself,
/// the parent of a file (with the file to put the cursor on), or the nearest
/// ancestor that is a directory. `None` when nothing above the path exists.
#[must_use]
pub fn resolve(path: &Path) -> Option<Resolved> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_dir() => Some(Resolved {
            dir: path.to_path_buf(),
            file: None,
        }),
        Ok(_) => Some(Resolved {
            dir: path.parent()?.to_path_buf(),
            file: path.file_name().map(OsString::from),
        }),
        Err(_) => path
            .ancestors()
            .skip(1)
            .find(|ancestor| fs::metadata(ancestor).is_ok_and(|metadata| metadata.is_dir()))
            .map(|ancestor| Resolved {
                dir: ancestor.to_path_buf(),
                file: None,
            }),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_directory_resolves_to_itself() {
        let dir = TempDir::new().unwrap();

        assert_eq!(
            resolve(dir.path()),
            Some(Resolved {
                dir: dir.path().to_path_buf(),
                file: None,
            })
        );
    }

    #[test]
    fn a_file_resolves_to_its_parent_with_the_file_name() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, b"").unwrap();

        assert_eq!(
            resolve(&file),
            Some(Resolved {
                dir: dir.path().to_path_buf(),
                file: Some(OsString::from("a.txt")),
            })
        );
    }

    #[test]
    fn a_missing_path_resolves_to_the_nearest_existing_directory() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();

        assert_eq!(
            resolve(&dir.path().join("sub").join("gone").join("deeper")),
            Some(Resolved {
                dir: dir.path().join("sub"),
                file: None,
            })
        );
    }

    #[test]
    fn a_path_below_a_file_skips_the_file() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("file"), b"").unwrap();

        assert_eq!(
            resolve(&dir.path().join("file").join("x")),
            Some(Resolved {
                dir: dir.path().to_path_buf(),
                file: None,
            })
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_broken_link_counts_as_missing() {
        use std::os::unix::fs::symlink;

        let dir = TempDir::new().unwrap();
        let link = dir.path().join("dead");
        symlink("missing", &link).unwrap();

        assert_eq!(
            resolve(&link),
            Some(Resolved {
                dir: dir.path().to_path_buf(),
                file: None,
            })
        );
    }
}
