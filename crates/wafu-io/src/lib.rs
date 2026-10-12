//! Effect runtime of the wafu file manager.
//!
//! Executes [`wafu_core::Effect`]s that touch the outside world and reports the
//! results back to the application as [`wafu_core::IoEvent`]s. Nothing here
//! knows about the UI.

mod read_dir;
mod runtime;
mod time;
mod watch;

pub use read_dir::read_dir;
pub use runtime::Runtime;
