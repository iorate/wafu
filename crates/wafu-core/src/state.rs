use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// What is saved between runs and handed to a new window.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StateFile {
    pub left: PaneState,
    pub right: PaneState,
    pub compare: CompareState,
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

/// What `mark_compare` last used.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CompareState {
    /// The condition by name (`missing`, `newer`, or `different`).
    pub condition: Option<String>,
    pub include_dirs: Option<bool>,
    pub ignore_case: Option<bool>,
}

/// The window's place on the screen. The size is in logical pixels; the
/// position is in physical pixels on Windows and in logical pixels elsewhere,
/// and is absent where the platform does not provide it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WindowState {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

/// A screen's rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
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

impl WindowState {
    /// The saved position, if the window placed there with the saved size
    /// shows at least 100 pixels (or its whole width or height, if smaller)
    /// each way on one of `screens`. `None` when the position or the size is
    /// not saved.
    #[must_use]
    pub fn position_on(&self, screens: &[ScreenRect]) -> Option<(i32, i32)> {
        let (Some(x), Some(y), Some(width), Some(height)) =
            (self.x, self.y, self.width, self.height)
        else {
            return None;
        };
        screens
            .iter()
            .any(|screen| {
                overlap(x, width, screen.x, screen.width) >= i64::from(width.min(100))
                    && overlap(y, height, screen.y, screen.height) >= i64::from(height.min(100))
            })
            .then_some((x, y))
    }
}

fn overlap(start: i32, length: u32, other_start: i32, other_length: u32) -> i64 {
    let end = i64::from(start) + i64::from(length);
    let other_end = i64::from(other_start) + i64::from(other_length);
    end.min(other_end) - i64::from(start).max(i64::from(other_start))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREENS: &[ScreenRect] = &[ScreenRect {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    }];

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
            compare: CompareState {
                condition: Some("newer".to_owned()),
                include_dirs: Some(false),
                ignore_case: Some(true),
            },
            window: Some(WindowState {
                x: Some(-10),
                y: Some(20),
                width: Some(960),
                height: Some(640),
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
    fn position_on_keeps_a_window_fully_on_a_screen() {
        assert_eq!(
            window(100, 100, 80, 60).position_on(SCREENS),
            Some((100, 100))
        );
    }

    #[test]
    fn position_on_keeps_a_window_enough_of_which_is_on_a_screen() {
        assert_eq!(
            window(1820, 100, 800, 600).position_on(SCREENS),
            Some((1820, 100))
        );
    }

    #[test]
    fn position_on_drops_a_window_too_little_of_which_is_on_a_screen() {
        assert_eq!(window(1821, 100, 800, 600).position_on(SCREENS), None);
    }

    #[test]
    fn position_on_drops_a_window_off_every_screen() {
        assert_eq!(window(100, 2000, 800, 600).position_on(SCREENS), None);
    }

    #[test]
    fn position_on_measures_from_the_edges_of_a_screen_away_from_the_origin() {
        let screens = &[ScreenRect {
            x: 1920,
            y: 200,
            width: 1280,
            height: 1024,
        }];
        assert_eq!(
            window(1220, -500, 800, 800).position_on(screens),
            Some((1220, -500))
        );
        assert_eq!(window(1219, -500, 800, 800).position_on(screens), None);
        assert_eq!(window(1220, -501, 800, 800).position_on(screens), None);
    }

    #[test]
    fn position_on_drops_the_position_without_screens() {
        assert_eq!(window(100, 100, 800, 600).position_on(&[]), None);
    }

    fn window(x: i32, y: i32, width: u32, height: u32) -> WindowState {
        WindowState {
            x: Some(x),
            y: Some(y),
            width: Some(width),
            height: Some(height),
        }
    }
}
