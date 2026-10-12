//! Effect runtime of the wafu file manager.
//!
//! Executes [`wafu_core::Effect`]s that touch the outside world and reports the
//! results back to the application as [`wafu_core::IoEvent`]s. Nothing here
//! knows about the UI.

mod drives;
mod read_dir;
mod resolve;
mod runtime;
mod time;
mod watch;

pub use drives::drives;
pub use read_dir::{read_dir, read_dir_with};
pub use resolve::resolve;
pub use runtime::Runtime;
