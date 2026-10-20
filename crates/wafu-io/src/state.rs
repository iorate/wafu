use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use wafu_core::{StateError, StateFile};

static HANDOFF_COUNT: AtomicU32 = AtomicU32::new(0);

/// Reads the state file. `Ok(None)` when there is none.
pub fn read_state(path: &Path) -> Result<Option<StateFile>, StateError> {
    match fs::read_to_string(path) {
        Ok(text) => StateFile::from_toml(&text).map(Some),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(StateError::new(error.to_string())),
    }
}

pub fn write_state(path: &Path, state: &StateFile) -> std::io::Result<()> {
    let text = state
        .to_toml()
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, text)
}

/// Writes the state for a new instance to a temporary file and returns its path.
pub fn write_handoff(state: &StateFile) -> std::io::Result<PathBuf> {
    let count = HANDOFF_COUNT.fetch_add(1, Ordering::Relaxed);
    let path =
        std::env::temp_dir().join(format!("wafu-inherit-{}-{count}.toml", std::process::id()));
    write_state(&path, state)?;
    Ok(path)
}

/// Reads a handoff file and deletes it, whether or not it could be read.
pub fn read_handoff(path: &Path) -> Result<Option<StateFile>, StateError> {
    let state = read_state(path);
    let _ = fs::remove_file(path);
    state
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
}
