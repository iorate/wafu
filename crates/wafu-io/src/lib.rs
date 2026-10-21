//! Effect runtime of the wafu file manager.
//!
//! Executes [`wafu_core::Effect`]s that touch the outside world and reports the
//! results back to the application as [`wafu_core::IoEvent`]s. Nothing here
//! knows about the UI.

mod archive;
mod config;
mod drives;
mod fs_ops;
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
mod watch;

pub use config::read_or_create;
pub use drives::drives;
pub use fs_ops::{create_file, mkdir, open_with_os, rename, set_clipboard};
pub use operation::run_operation;
pub use paths::{config_path, is_wayland, state_path};
pub use preview::{decode_image, load_image, load_preview};
pub use read_dir::{read_dir, read_dir_with};
pub use resolve::resolve;
pub use runtime::{Paths, Runtime};
#[cfg(windows)]
pub use shell_menu::{ShellMenuError, shell_menu};
pub use spawn::{spawn, spawn_new_window};
pub use state::{read_handoff, read_state, write_handoff, write_state};
