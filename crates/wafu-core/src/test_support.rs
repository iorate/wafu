use std::ffi::OsString;
use std::path::PathBuf;

use crate::app::{App, Effect, IoEvent, Msg, Side};
use crate::entry::{Entry, EntryKind};
use crate::error::IoError;
use crate::key::{Key, KeyBase, KeyInput};
use crate::pane::RequestId;
use crate::view::View;

/// Drives an [`App`] with messages and records the effects, so that scenario
/// tests read like a script: press a key, answer an I/O request, look at the view.
pub(crate) struct Harness {
    app: App,
    effects: Vec<Effect>,
    requests: [Option<RequestId>; 2],
}

impl Harness {
    pub(crate) fn start(left: &str, right: &str) -> Self {
        let mut harness = Self {
            app: App::new(),
            effects: Vec::new(),
            requests: [None, None],
        };
        harness.send(Msg::Start {
            left: PathBuf::from(left),
            right: PathBuf::from(right),
        });
        harness
    }

    pub(crate) fn send(&mut self, msg: Msg) {
        let effects = self.app.update(msg);
        for effect in &effects {
            if let Effect::ReadDir { side, request, .. } = effect {
                self.requests[index(*side)] = Some(*request);
            }
        }
        self.effects.extend(effects);
    }

    /// Presses a key written as in the configuration, simulating what the UI would deliver.
    pub(crate) fn press(&mut self, key: &str) {
        let key: Key = key.parse().expect("test key should parse");
        let text = match key.base {
            KeyBase::Char(c) if c.is_ascii_alphabetic() && !key.shift => {
                c.to_ascii_lowercase().to_string()
            }
            KeyBase::Char(c) => c.to_string(),
            KeyBase::Named(named) => named.to_string(),
        };
        self.send(Msg::Key(KeyInput {
            text,
            ctrl: key.ctrl,
            shift: key.shift,
            alt: key.alt,
        }));
    }

    pub(crate) fn viewport(&mut self, side: Side, rows: usize) {
        self.send(Msg::ViewportRows { side, rows });
    }

    /// Answers the latest `ReadDir` request of a side.
    pub(crate) fn load(&mut self, side: Side, entries: Vec<Entry>) {
        let request = self.latest_request(side);
        self.send(Msg::Io(IoEvent::DirLoaded {
            side,
            request,
            result: Ok(entries),
        }));
    }

    pub(crate) fn fail_load(&mut self, side: Side, message: &str) {
        let request = self.latest_request(side);
        self.send(Msg::Io(IoEvent::DirLoaded {
            side,
            request,
            result: Err(IoError::new(message)),
        }));
    }

    pub(crate) fn latest_request(&self, side: Side) -> RequestId {
        self.requests[index(side)].expect("a ReadDir request should have been issued")
    }

    pub(crate) fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub(crate) fn app(&self) -> &App {
        &self.app
    }

    pub(crate) fn view(&self) -> View {
        self.app.view()
    }

    pub(crate) fn names(&self, side: Side) -> Vec<String> {
        let view = self.view();
        let pane = match side {
            Side::Left => view.left,
            Side::Right => view.right,
        };
        pane.rows
            .iter()
            .map(|row| {
                if row.extension.is_empty() {
                    row.name.clone()
                } else {
                    format!("{}.{}", row.name, row.extension)
                }
            })
            .collect()
    }

    pub(crate) fn cursor_name(&self, side: Side) -> String {
        let pane = self.app.pane(side);
        pane.cursor_entry().map_or_else(
            || "..".to_owned(),
            |entry| entry.name.to_string_lossy().into_owned(),
        )
    }
}

pub(crate) fn file(name: &str) -> Entry {
    Entry {
        name: OsString::from(name),
        kind: EntryKind::File,
        size: 0,
        modified: None,
        hidden: false,
    }
}

pub(crate) fn dir(name: &str) -> Entry {
    Entry {
        kind: EntryKind::Dir,
        ..file(name)
    }
}

fn index(side: Side) -> usize {
    match side {
        Side::Left => 0,
        Side::Right => 1,
    }
}
