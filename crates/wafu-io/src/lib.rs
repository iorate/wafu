//! Effect runtime of the wafu file manager.
//!
//! Executes [`wafu_core::Effect`]s that touch the outside world and reports the
//! results back to the application. Nothing here knows about the UI.

/// Executes effects on behalf of the application.
#[derive(Debug, Default)]
pub struct Runtime;

impl Runtime {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}
