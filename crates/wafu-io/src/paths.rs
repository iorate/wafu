use std::path::PathBuf;

/// The configuration file in the user's local (non-roaming) configuration
/// directory, or `None` when the platform does not say where that is.
#[must_use]
pub fn config_path() -> Option<PathBuf> {
    dirs::config_local_dir().map(|dir| dir.join("wafu").join("config.toml"))
}

/// The state file in the user's state directory where the platform has one,
/// else in the local data directory.
#[must_use]
pub fn state_path() -> Option<PathBuf> {
    dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .map(|dir| dir.join("wafu").join("state.toml"))
}

/// Whether the display is Wayland, which does not let a window learn or set
/// its position.
#[must_use]
pub fn is_wayland() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty())
}
