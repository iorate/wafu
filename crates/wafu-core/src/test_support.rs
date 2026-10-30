use std::ffi::OsString;
use std::path::PathBuf;

use crate::app::{
    App, Change, ConfigSource, DrivePurpose, Effect, IoEvent, MouseInput, Msg, Os, PreviewError,
    Resolved, Side, Start,
};
use crate::entry::{Directory, Entry, EntryKind, SpecialKind};
use crate::error::IoError;
use crate::key::{Key, KeyBase, KeyInput};
use crate::operation::{OperationEvent, OperationSpec, Outcome};
use crate::pane::RequestId;
use crate::popup::Popup;
use crate::preview::{Document, ImageData};
use crate::state::{StateError, StateFile};
use crate::view::View;

pub(crate) const HOME: &str = "/home/user";
pub(crate) const CONFIG_PATH: &str = "/home/user/.config/wafu/config.toml";

/// Drives an [`App`] with messages and records the effects, so that scenario
/// tests read like a script: press a key, answer an I/O request, look at the view.
pub(crate) struct Harness {
    app: App,
    effects: Vec<Effect>,
    requests: [Option<RequestId>; 2],
    resolves: [Option<RequestId>; 2],
    preview_request: Option<RequestId>,
}

impl Harness {
    /// Starts on Linux with both panes loaded empty, so that keys work at once.
    pub(crate) fn start(left: &str, right: &str) -> Self {
        Self::start_on(Os::Linux, left, right)
    }

    pub(crate) fn start_on(os: Os, left: &str, right: &str) -> Self {
        let mut harness = Self::start_both_pending(os, left, right);
        harness.load(Side::Left, Vec::new());
        harness.load(Side::Right, Vec::new());
        harness
    }

    /// Starts with both panes waiting for their listings.
    pub(crate) fn start_both_pending(os: Os, left: &str, right: &str) -> Self {
        let mut harness = Self::start_unresolved(os, left, right);
        harness.resolve(Side::Left, left, None);
        harness.resolve(Side::Right, right, None);
        harness
    }

    /// Starts without answering the resolves, for tests of the start-up itself.
    pub(crate) fn start_unresolved(os: Os, left: &str, right: &str) -> Self {
        Self::start_from(start_msg(os, Some(left), Some(right)))
    }

    /// Starts from a `Start` message built by [`start_msg`] and changed as the
    /// test needs, without answering anything.
    pub(crate) fn start_from(msg: Msg) -> Self {
        let mut harness = Self {
            app: App::new(),
            effects: Vec::new(),
            requests: [None, None],
            resolves: [None, None],
            preview_request: None,
        };
        harness.send(msg);
        harness
    }

    pub(crate) fn send(&mut self, msg: Msg) {
        let effects = self.app.update(msg);
        for effect in &effects {
            match effect {
                Effect::ReadDir { side, request, .. } => {
                    self.requests[side.index()] = Some(*request);
                }
                Effect::Resolve { side, request, .. } => {
                    self.resolves[side.index()] = Some(*request);
                }
                Effect::LoadPreview { request, .. } | Effect::LoadImage { request, .. } => {
                    self.preview_request = Some(*request);
                }
                _ => {}
            }
        }
        self.effects.extend(effects);
    }

    /// Reloads the configuration from text, as the watch does after an edit.
    pub(crate) fn config(&mut self, text: &str) {
        self.send(Msg::Io(IoEvent::ConfigLoaded {
            result: Ok(text.to_owned()),
        }));
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

    /// Edits the popup's first input field, as the UI does after every keystroke.
    pub(crate) fn edit(&mut self, text: &str) {
        self.edit_field(0, text);
    }

    pub(crate) fn edit_field(&mut self, field: usize, text: &str) {
        self.send(Msg::InputChanged {
            field,
            text: text.to_owned(),
        });
    }

    pub(crate) fn click(&mut self, side: Side, row: usize) {
        self.send(Msg::Mouse(MouseInput::Click { side, row }));
    }

    pub(crate) fn double_click(&mut self, side: Side, row: usize) {
        self.send(Msg::Mouse(MouseInput::Click { side, row }));
        self.send(Msg::Mouse(MouseInput::DoubleClick { side, row }));
    }

    pub(crate) fn wheel(&mut self, side: Side, rows: i32) {
        self.send(Msg::Mouse(MouseInput::Wheel { side, rows }));
    }

    pub(crate) fn scroll_to(&mut self, side: Side, top: usize) {
        self.send(Msg::Mouse(MouseInput::ScrollTo { side, top }));
    }

    pub(crate) fn viewport(&mut self, side: Side, rows: usize) {
        self.send(Msg::ViewportRows { side, rows });
    }

    pub(crate) fn preview_rows(&mut self, rows: usize) {
        self.send(Msg::PreviewRows { rows });
    }

    pub(crate) fn preview_double_click(&mut self) {
        self.send(Msg::Mouse(MouseInput::PreviewDoubleClick));
    }

    pub(crate) fn preview_wheel(&mut self, rows: i32) {
        self.send(Msg::Mouse(MouseInput::PreviewWheel { rows }));
    }

    /// Answers the latest preview request with a decoded document.
    pub(crate) fn preview_loaded(&mut self, document: Document) {
        let request = self.latest_preview_request();
        self.send(Msg::Io(IoEvent::PreviewLoaded {
            request,
            result: Ok(document),
        }));
    }

    pub(crate) fn fail_preview(&mut self, error: PreviewError) {
        let request = self.latest_preview_request();
        self.send(Msg::Io(IoEvent::PreviewLoaded {
            request,
            result: Err(error),
        }));
    }

    /// Answers the latest image request with a blank image of the given size.
    pub(crate) fn image_loaded(&mut self, width: u32, height: u32) {
        let request = self.latest_preview_request();
        self.send(Msg::Io(IoEvent::ImageLoaded {
            request,
            result: Ok(ImageData {
                width,
                height,
                rgba: vec![0; (width * height * 4) as usize],
                format: "png",
            }),
        }));
    }

    pub(crate) fn fail_image(&mut self, error: PreviewError) {
        let request = self.latest_preview_request();
        self.send(Msg::Io(IoEvent::ImageLoaded {
            request,
            result: Err(error),
        }));
    }

    pub(crate) fn latest_preview_request(&self) -> RequestId {
        self.preview_request
            .expect("a LoadPreview or LoadImage request should have been issued")
    }

    /// The texts of the lines a text or binary preview shows.
    pub(crate) fn preview_lines(&self) -> Vec<String> {
        self.preview_content()
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    /// The line numbers beside the lines a preview shows.
    pub(crate) fn preview_numbers(&self) -> Vec<String> {
        self.preview_content()
            .into_iter()
            .map(|line| line.number)
            .collect()
    }

    fn preview_content(&self) -> Vec<crate::view::PreviewLine> {
        match self.view().preview.map(|preview| preview.content) {
            Some(crate::view::PreviewContent::Lines { lines, .. }) => lines,
            other => panic!("expected a lines preview, got {other:?}"),
        }
    }

    /// Delivers a listing to a side: answers its pending read, or, when none
    /// is pending, reports a change first so that the side reads again.
    pub(crate) fn load(&mut self, side: Side, entries: Vec<Entry>) {
        self.load_directory(side, Directory { own: None, entries });
    }

    /// Like `load`, with the location itself as an entry.
    pub(crate) fn load_directory(&mut self, side: Side, directory: Directory) {
        if self.app.pane(side).loading().is_none() {
            self.changed(side);
        }
        let request = self.latest_request(side);
        self.send(Msg::Io(IoEvent::DirLoaded {
            side,
            request,
            result: Ok(directory),
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

    /// Answers the latest `Resolve` request of a side with a directory and,
    /// optionally, a file to put the cursor on.
    pub(crate) fn resolve(&mut self, side: Side, dir: &str, file: Option<&str>) {
        let request = self.latest_resolve(side);
        self.send(Msg::Io(IoEvent::Resolved {
            side,
            request,
            result: Some(Resolved {
                dir: PathBuf::from(dir),
                file: file.map(OsString::from),
            }),
        }));
    }

    /// Answers the latest `Resolve` request of a side with "nothing exists".
    pub(crate) fn fail_resolve(&mut self, side: Side) {
        let request = self.latest_resolve(side);
        self.send(Msg::Io(IoEvent::Resolved {
            side,
            request,
            result: None,
        }));
    }

    /// Reports something from the operation worker.
    pub(crate) fn operation(&mut self, event: OperationEvent) {
        self.send(Msg::Io(IoEvent::Operation(event)));
    }

    pub(crate) fn finish(&mut self, outcome: Outcome) {
        self.operation(OperationEvent::Finished(outcome));
    }

    /// Enters the archive under the cursor with Enter and answers the read
    /// with the given entries.
    pub(crate) fn right_click(&mut self, side: Side, row: usize) {
        self.send(Msg::Mouse(MouseInput::RightClick { side, row }));
    }

    pub(crate) fn enter_archive(&mut self, side: Side, entries: Vec<Entry>) {
        self.press("Enter");
        self.load(side, entries);
    }

    /// Takes the `StartOperation` out of the recorded effects.
    pub(crate) fn start_operation(&mut self) -> Option<OperationSpec> {
        self.take_effects()
            .into_iter()
            .find_map(|effect| match effect {
                Effect::StartOperation(spec) => Some(spec),
                _ => None,
            })
    }

    /// Reports the result of a rename or a creation.
    pub(crate) fn changed_result(&mut self, change: Change, error: Option<&str>) {
        self.send(Msg::Io(IoEvent::Changed {
            change,
            error: error.map(IoError::new),
        }));
    }

    pub(crate) fn drives(&mut self, purpose: DrivePurpose, letters: &[char]) {
        self.send(Msg::Io(IoEvent::Drives {
            purpose,
            letters: letters.to_vec(),
        }));
    }

    /// Reports a change in the directory a side is showing.
    pub(crate) fn changed(&mut self, side: Side) {
        let path = self.app.pane(side).location().base.clone();
        self.send(Msg::Io(IoEvent::LocationChanged { side, path }));
    }

    pub(crate) fn latest_request(&self, side: Side) -> RequestId {
        self.requests[side.index()].expect("a ReadDir request should have been issued")
    }

    pub(crate) fn latest_resolve(&self, side: Side) -> RequestId {
        self.resolves[side.index()].expect("a Resolve request should have been issued")
    }

    pub(crate) fn take_effects(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    pub(crate) fn app(&self) -> &App {
        &self.app
    }

    pub(crate) fn popup(&self) -> Option<&Popup> {
        self.app.popup()
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
            .map(|row| format!("{}{}", row.name, row.extension))
            .collect()
    }

    pub(crate) fn marks(&self, side: Side) -> Vec<String> {
        self.app
            .pane(side)
            .marked_entries()
            .map(|entry| entry.name.to_string_lossy().into_owned())
            .collect()
    }

    pub(crate) fn cursor_name(&self, side: Side) -> String {
        let pane = self.app.pane(side);
        pane.cursor_entry().map_or_else(
            || "..".to_owned(),
            |entry| entry.name.to_string_lossy().into_owned(),
        )
    }

    pub(crate) fn log(&self) -> Vec<String> {
        self.view()
            .log
            .lines
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    pub(crate) fn history(&self, side: Side) -> Vec<String> {
        self.app
            .pane(side)
            .history()
            .items()
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    }

    /// The texts of the popup's input fields.
    pub(crate) fn popup_fields(&self) -> Vec<String> {
        self.view()
            .popup
            .map(|popup| popup.fields.into_iter().map(|field| field.text).collect())
            .unwrap_or_default()
    }

    pub(crate) fn popup_labels(&self) -> Vec<String> {
        self.view()
            .popup
            .map(|popup| popup.items.into_iter().map(|item| item.label).collect())
            .unwrap_or_default()
    }
}

/// A `Start` on the given OS with an empty configuration and no state file.
pub(crate) fn start_msg(os: Os, left: Option<&str>, right: Option<&str>) -> Msg {
    start_msg_with(os, left, right, ConfigSource::Text(String::new()), Ok(None))
}

pub(crate) fn start_msg_with(
    os: Os,
    left: Option<&str>,
    right: Option<&str>,
    config: ConfigSource,
    state: Result<Option<StateFile>, StateError>,
) -> Msg {
    Msg::Start(Box::new(Start {
        left: left.map(PathBuf::from),
        right: right.map(PathBuf::from),
        home: PathBuf::from(HOME),
        os,
        config_path: PathBuf::from(CONFIG_PATH),
        config,
        state,
    }))
}

pub(crate) fn file(name: &str) -> Entry {
    Entry {
        name: OsString::from(name),
        kind: EntryKind::File { executable: false },
        size: 0,
        modified: None,
        hidden: false,
        permissions: None,
        link_target: None,
    }
}

pub(crate) fn dir(name: &str) -> Entry {
    Entry {
        kind: EntryKind::Dir,
        ..file(name)
    }
}

pub(crate) fn special(name: &str, kind: SpecialKind) -> Entry {
    Entry {
        kind: EntryKind::Special(kind),
        ..file(name)
    }
}
