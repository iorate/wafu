use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// What is saved between runs and handed to a new window.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StateFile {
    pub left: PaneState,
    pub right: PaneState,
    pub window: Option<WindowState>,
    pub split_percent: Option<u8>,
    pub log_rows: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PaneState {
    pub path: Option<PathBuf>,
    /// The sort key by name (`name`, `size`, ...).
    pub sort: Option<String>,
    pub descending: Option<bool>,
    pub dirs_first: Option<bool>,
    pub ignore_case: Option<bool>,
    pub show_hidden: Option<bool>,
    pub history: Vec<PathBuf>,
    pub history_pos: Option<usize>,
}

/// The window's place on the screen in logical pixels. The position is
/// absent where the platform does not provide it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct StateError(String);

impl StateError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl StateFile {
    pub fn from_toml(text: &str) -> Result<Self, StateError> {
        toml::from_str(text).map_err(|error| StateError(error.message().to_owned()))
    }

    pub fn to_toml(&self) -> Result<String, StateError> {
        toml::to_string(self).map_err(|error| StateError(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_toml() {
        let state = StateFile {
            left: PaneState {
                path: Some(PathBuf::from("/home/user")),
                sort: Some("size".to_owned()),
                descending: Some(true),
                dirs_first: Some(false),
                ignore_case: Some(true),
                show_hidden: Some(true),
                history: vec![PathBuf::from("/"), PathBuf::from("/home/user")],
                history_pos: Some(1),
            },
            right: PaneState::default(),
            window: Some(WindowState {
                x: Some(-10),
                y: Some(20),
                width: 960,
                height: 640,
            }),
            split_percent: Some(60),
            log_rows: Some(8),
        };

        let text = state.to_toml().unwrap();

        assert_eq!(StateFile::from_toml(&text), Ok(state));
    }

    #[test]
    fn missing_items_take_their_defaults() {
        let state = StateFile::from_toml("[left]\npath = \"/tmp\"\n").unwrap();

        assert_eq!(state.left.path, Some(PathBuf::from("/tmp")));
        assert_eq!(state.left.sort, None);
        assert_eq!(state.right, PaneState::default());
        assert_eq!(state.window, None);
        assert_eq!(StateFile::from_toml(""), Ok(StateFile::default()));
    }

    #[test]
    fn a_window_without_a_position_round_trips() {
        let state = StateFile {
            window: Some(WindowState {
                x: None,
                y: None,
                width: 1,
                height: 2,
            }),
            ..StateFile::default()
        };

        let text = state.to_toml().unwrap();

        assert_eq!(StateFile::from_toml(&text), Ok(state));
    }

    #[test]
    fn bad_toml_is_an_error() {
        assert!(StateFile::from_toml("[left\n").is_err());
        assert!(StateFile::from_toml("[left]\npath = 1\n").is_err());
    }
}
