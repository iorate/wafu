//! Effect runtime of the wafu file manager.
//!
//! Executes [`wafu_core::Effect`]s that touch the outside world and reports the
//! results back to the application as [`wafu_core::IoEvent`]s. Nothing here
//! knows about the UI.

mod archive;
#[cfg(windows)]
mod color_scheme;
#[cfg(windows)]
mod com;
mod config;
mod drives;
mod fs_ops;
#[cfg(windows)]
mod message;
mod operation;
mod paths;
mod preview;
mod read_dir;
mod resolve;
mod runtime;
#[cfg(windows)]
mod shell_menu;
mod spawn;
mod state;
mod time;
mod trash;
mod watch;
#[cfg(windows)]
mod window_icon;
#[cfg(windows)]
mod window_place;

pub use config::read_or_create;
#[cfg(windows)]
pub use message::show_message;
pub use paths::{config_path, is_wayland, state_path};
pub use runtime::{Paths, Runtime};
#[cfg(windows)]
pub use shell_menu::{ShellMenuError, shell_menu};
pub use state::{read_handoff, read_state};
#[cfg(windows)]
pub use window_icon::set_small_icon;
#[cfg(windows)]
pub use window_place::{WindowPlace, normal_window_place};
