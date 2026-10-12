//! Pure model of the wafu file manager.
//!
//! The application state lives in [`App`]. Everything that happens is fed in as
//! a [`Msg`]; everything the application wants done to the outside world comes
//! back out as an [`Effect`]. This crate performs no I/O of its own.

#![forbid(unsafe_code)]

use std::path::PathBuf;

/// An event delivered to the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// The application has started with the given initial pane locations.
    Start { left: PathBuf, right: PathBuf },
}

/// A request from the application to the outside world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Set the window title.
    SetTitle(String),
}

/// A read-only snapshot of the state for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub title: String,
}

/// The whole application state.
#[derive(Debug, Default)]
pub struct App {
    left: Option<PathBuf>,
    right: Option<PathBuf>,
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies a message and returns the effects it produced.
    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        match msg {
            Msg::Start { left, right } => {
                self.left = Some(left);
                self.right = Some(right);
                vec![Effect::SetTitle(self.title())]
            }
        }
    }

    #[must_use]
    pub fn view(&self) -> View {
        View {
            title: self.title(),
        }
    }

    fn title(&self) -> String {
        match &self.left {
            Some(path) => format!("wafu - {}", path.display()),
            None => "wafu".to_owned(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_sets_title_to_left_pane_path() {
        let mut app = App::new();

        let effects = app.update(Msg::Start {
            left: PathBuf::from("/home/user"),
            right: PathBuf::from("/tmp"),
        });

        assert_eq!(
            effects,
            vec![Effect::SetTitle("wafu - /home/user".to_owned())]
        );
        assert_eq!(app.view().title, "wafu - /home/user");
    }
}
