use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};

use wafu_core::{StateError, StateFile};

const HANDOFF_PREFIX: &str = "wafu-inherit-";
const HANDOFF_SUFFIX: &str = ".toml";

/// Reads the state file. `Ok(None)` when there is none.
pub fn read_state(path: &Path) -> Result<Option<StateFile>, StateError> {
    match fs::read_to_string(path) {
        Ok(text) => StateFile::from_toml(&text).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(StateError::new(error.to_string())),
    }
}

/// Reads a handoff file and deletes it, whether or not it could be read.
pub fn read_handoff(path: &Path) -> Result<Option<StateFile>, StateError> {
    let state = read_state(path);
    let _ = fs::remove_file(path);
    state
}

// Written next to the file and moved over it, so that a crash while writing,
// or two instances quitting at once, cannot leave a broken file.
pub(crate) fn write_state(path: &Path, state: &StateFile) -> std::io::Result<()> {
    let text = to_toml(state)?;
    let dir = path.parent().unwrap_or(Path::new("."));
    fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(text.as_bytes())?;
    file.persist(path)?;
    Ok(())
}

// The temporary directory is shared with other users on Linux, so the file is
// created exclusively (a name made beforehand cannot be slipped in) and, on
// Unix, readable by its owner alone.
pub(crate) fn write_handoff(state: &StateFile) -> std::io::Result<PathBuf> {
    let text = to_toml(state)?;
    let mut file = tempfile::Builder::new()
        .prefix(HANDOFF_PREFIX)
        .suffix(HANDOFF_SUFFIX)
        .tempfile_in(std::env::temp_dir())?;
    file.write_all(text.as_bytes())?;
    let (_, path) = file.keep()?;
    Ok(path)
}

fn to_toml(state: &StateFile) -> std::io::Result<String> {
    state
        .to_toml()
        .map_err(|error| std::io::Error::other(error.to_string()))
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;
    use wafu_core::PaneState;

    use super::*;

    #[test]
    fn state_round_trips_and_is_absent_before_the_first_write() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("sub").join("state.toml");
        let state = StateFile {
            left: PaneState {
                path: Some(PathBuf::from("/a")),
                ..PaneState::default()
            },
            split_percent: Some(40),
            ..StateFile::default()
        };

        assert_eq!(read_state(&path), Ok(None));
        write_state(&path, &state).unwrap();

        assert_eq!(read_state(&path), Ok(Some(state)));
    }

    #[test]
    fn state_write_replaces_the_existing_file_and_leaves_no_temporary_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.toml");
        let state = StateFile {
            log_rows: Some(7),
            ..StateFile::default()
        };
        fs::write(&path, "[left\n").unwrap();

        write_state(&path, &state).unwrap();

        assert_eq!(read_state(&path), Ok(Some(state)));
        let names: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, ["state.toml"]);
    }

    #[test]
    fn a_broken_state_file_is_an_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("state.toml");
        fs::write(&path, "[left\n").unwrap();

        assert!(read_state(&path).is_err());
    }

    #[test]
    fn handoff_is_read_once_and_removed() {
        let state = StateFile {
            log_rows: Some(9),
            ..StateFile::default()
        };

        let path = write_handoff(&state).unwrap();

        assert_eq!(read_handoff(&path), Ok(Some(state)));
        assert!(!path.exists());
        assert_eq!(read_handoff(&path), Ok(None));
    }

    #[test]
    fn handoff_file_has_a_fresh_name_in_the_temporary_directory() {
        let state = StateFile::default();

        let first = write_handoff(&state).unwrap();
        let second = write_handoff(&state).unwrap();

        for path in [&first, &second] {
            let name = path.file_name().unwrap().to_str().unwrap();
            assert!(name.starts_with(HANDOFF_PREFIX), "{name}");
            assert!(name.ends_with(HANDOFF_SUFFIX), "{name}");
            assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
        }
        assert_ne!(first, second);
        let _ = read_handoff(&first);
        let _ = read_handoff(&second);
    }

    #[cfg(unix)]
    #[test]
    fn handoff_file_is_readable_only_by_its_owner() {
        use std::os::unix::fs::PermissionsExt;

        let path = write_handoff(&StateFile::default()).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        let _ = read_handoff(&path);

        assert_eq!(mode & 0o777, 0o600);
    }
}
