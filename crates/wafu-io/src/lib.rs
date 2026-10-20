//! Effect runtime of the wafu file manager.
//!
//! Executes [`wafu_core::Effect`]s that touch the outside world and reports the
//! results back to the application as [`wafu_core::IoEvent`]s. Nothing here
//! knows about the UI.

mod config;
mod drives;
mod paths;
mod read_dir;
mod resolve;
mod runtime;
mod spawn;
mod state;
mod time;
mod watch;

pub use config::read_or_create;
pub use drives::drives;
pub use paths::{config_path, is_wayland, state_path};
pub use read_dir::{read_dir, read_dir_with};
pub use resolve::resolve;
pub use runtime::{Paths, Runtime};
pub use spawn::{spawn, spawn_new_window};
pub use state::{read_handoff, read_state, write_handoff, write_state};
