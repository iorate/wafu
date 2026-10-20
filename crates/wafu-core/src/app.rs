use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use crate::command::{Command, Context};
use crate::config::Config;
use crate::entry::Entry;
use crate::error::{AppError, IoError};
use crate::key::{Key, KeyBase, KeyInput, NamedKey};
use crate::location::Location;
use crate::log::Log;
use crate::pane::{HistoryMove, Pane, RequestId};
use crate::popup::{InputKind, Popup};
use crate::state::{StateError, StateFile};
use crate::view::{LogView, PaneView, Style, View};

mod navigation;
mod popups;
mod session;
#[cfg(test)]
mod tests;

const SPLIT_STEP: u8 = 5;
const SPLIT_MIN: u8 = 10;
const SPLIT_MAX: u8 = 90;
const LOG_ROWS_DEFAULT: usize = 6;
const LOG_ROWS_MIN: usize = 1;
const LOG_ROWS_MAX: usize = 30;

/// An event delivered to the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// The application has started.
    Start(Box<Start>),
    /// The window's close button, or the Quit item of the menu bar.
    CloseRequested,
    Key(KeyInput),
    Mouse(MouseInput),
    /// The text of the popup's input field was edited.
    InputChanged(String),
    /// The number of rows that fit in a pane's list.
    ViewportRows {
        side: Side,
        rows: usize,
    },
    Io(IoEvent),
}

/// A mouse action on a pane. `row` is the display row under the pointer and
/// may be past the last row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseInput {
    Click {
        side: Side,
        row: usize,
    },
    DoubleClick {
        side: Side,
        row: usize,
    },
    RightClick {
        side: Side,
        row: usize,
    },
    /// Wheel movement in rows; positive scrolls down.
    Wheel {
        side: Side,
        rows: i32,
    },
    /// Wheel movement over the log, in rows.
    LogWheel {
        rows: i32,
    },
}

/// What the application starts from. The pane paths need not exist yet; the
/// nearest existing directory is used. A pane without a path on the command
/// line takes the saved one, or home.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    pub left: Option<PathBuf>,
    pub right: Option<PathBuf>,
    pub home: PathBuf,
    pub os: Os,
    pub config_path: PathBuf,
    pub config: ConfigSource,
    /// `Ok(None)` when there is no state file.
    pub state: Result<Option<StateFile>, StateError>,
}

/// How the configuration file was found at start-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    Text(String),
    /// There was none, so the template was written.
    Created,
    Unreadable(IoError),
}

/// A request from the application to the outside world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ReadDir {
        side: Side,
        location: Location,
        request: RequestId,
    },
    /// Finds an existing directory at or above the path.
    Resolve {
        side: Side,
        path: PathBuf,
        request: RequestId,
    },
    /// Cancels a `ReadDir` or `Resolve`.
    CancelRead {
        request: RequestId,
    },
    /// Watches a directory for changes, replacing the side's previous watch.
    Watch {
        side: Side,
        path: PathBuf,
    },
    /// Lists the drive letters (Windows only).
    ListDrives {
        purpose: DrivePurpose,
    },
    SetTitle(String),
    /// Watches the configuration file and reports its new text on every change.
    WatchConfig(PathBuf),
    /// Starts a program without a shell and without waiting for it.
    Spawn {
        program: String,
        args: Vec<OsString>,
        cwd: PathBuf,
    },
    /// Writes the state file. The window geometry is filled in by the UI,
    /// which is the only layer that knows it.
    SaveState(StateFile),
    /// Starts another instance that takes over the state. The window
    /// geometry is filled in by the UI, as for `SaveState`.
    SpawnNewWindow(StateFile),
    Quit,
}

/// What `Effect::Spawn` or `Effect::SpawnNewWindow` started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnKind {
    External { program: String },
    NewWindow,
}

/// What the drive list is for; it comes back with the reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrivePurpose {
    Show,
    Prev,
    Next,
}

/// The answer to [`Effect::Resolve`]: the directory to open and, when the
/// path named a file, the file to put the cursor on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub dir: PathBuf,
    pub file: Option<OsString>,
}

/// The operating system, passed in rather than detected so that the core
/// stays independent of the environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

/// The result of an effect, reported by the I/O layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoEvent {
    DirLoaded {
        side: Side,
        request: RequestId,
        result: Result<Vec<Entry>, IoError>,
    },
    /// `None` when no directory exists anywhere above the path.
    Resolved {
        side: Side,
        request: RequestId,
        result: Option<Resolved>,
    },
    /// Something changed in the watched directory.
    LocationChanged {
        side: Side,
        path: PathBuf,
    },
    /// Always empty except on Windows.
    Drives {
        purpose: DrivePurpose,
        letters: Vec<char>,
    },
    WatchFailed {
        side: Side,
        path: PathBuf,
        error: IoError,
    },
    /// The configuration file changed and was read again.
    ConfigLoaded {
        result: Result<String, IoError>,
    },
    ConfigWatchFailed {
        error: IoError,
    },
    Spawned {
        kind: SpawnKind,
        error: Option<IoError>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// The whole application state.
#[derive(Debug)]
pub struct App {
    panes: [Pane; 2],
    active: Side,
    config: Config,
    config_path: PathBuf,
    style: Arc<Style>,
    log: Log,
    popup: Option<Popup>,
    split_percent: u8,
    next_request: u64,
    home: PathBuf,
    os: Os,
}

impl Side {
    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    /// `0` for the left side and `1` for the right, for indexing a pair.
    #[must_use]
    pub fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        let mut log = Log::new();
        log.set_viewport_rows(LOG_ROWS_DEFAULT);
        let config = Config::default_for(Os::Linux);
        Self {
            panes: [
                Pane::new(Location::new(PathBuf::new())),
                Pane::new(Location::new(PathBuf::new())),
            ],
            active: Side::Left,
            style: Arc::new(Style::from_config(&config)),
            config,
            config_path: PathBuf::new(),
            log,
            popup: None,
            split_percent: 50,
            next_request: 1,
            home: PathBuf::new(),
            os: Os::Linux,
        }
    }

    /// Applies a message and returns the effects it produced.
    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        let title_before = self.title();
        let mut effects = Vec::new();
        match msg {
            Msg::Start(start) => {
                let Start {
                    left,
                    right,
                    home,
                    os,
                    config_path,
                    config,
                    state,
                } = *start;
                self.home = home;
                self.os = os;
                self.config_path = config_path;
                self.panes = [
                    Pane::new(Location::new(PathBuf::new())),
                    Pane::new(Location::new(PathBuf::new())),
                ];
                self.start(left, right, config, state, &mut effects);
                effects.push(Effect::SetTitle(self.title()));
                return effects;
            }
            Msg::CloseRequested => self.quit(true, &mut effects),
            Msg::Key(input) => {
                if let Some(key) = Key::from_input(&input) {
                    if self.popup.is_some() {
                        self.popup_key(key, &mut effects);
                    } else if self.navigating() {
                        // A move in progress is modal: only Esc, which cancels it, gets through.
                        if key.base == KeyBase::Named(NamedKey::Escape) {
                            self.cancel_navigations(&mut effects);
                        }
                    } else {
                        if let Some(command) = self.config.keys.lookup(Context::FileList, &key) {
                            self.run_command(command, &mut effects);
                        }
                        // A key after wheel scrolling brings the cursor back into view.
                        self.active_pane_mut().ensure_cursor_visible();
                    }
                }
            }
            Msg::Mouse(input) => {
                if self.popup.is_none() && !self.navigating() {
                    self.mouse(input, &mut effects);
                }
            }
            Msg::InputChanged(text) => self.input_changed(text),
            Msg::ViewportRows { side, rows } => self.panes[side.index()].set_viewport_rows(rows),
            Msg::Io(IoEvent::DirLoaded {
                side,
                request,
                result,
            }) => self.dir_loaded(side, request, result, &mut effects),
            Msg::Io(IoEvent::Resolved {
                side,
                request,
                result,
            }) => self.resolved(side, request, result, &mut effects),
            Msg::Io(IoEvent::LocationChanged { side, path }) => {
                self.location_changed(side, &path, &mut effects);
            }
            Msg::Io(IoEvent::Drives { purpose, letters }) => {
                self.drives(purpose, letters, &mut effects);
            }
            Msg::Io(IoEvent::WatchFailed { path, error, .. }) => {
                self.log.push(
                    AppError::Watch {
                        path,
                        source: error,
                    }
                    .to_string(),
                );
            }
            Msg::Io(IoEvent::ConfigLoaded { result }) => self.config_loaded(result),
            Msg::Io(IoEvent::ConfigWatchFailed { error }) => {
                self.log.push(
                    AppError::Watch {
                        path: self.config_path.clone(),
                        source: error,
                    }
                    .to_string(),
                );
            }
            Msg::Io(IoEvent::Spawned { kind, error }) => self.spawned(kind, error),
        }
        let title = self.title();
        if title != title_before {
            effects.push(Effect::SetTitle(title));
        }
        effects
    }

    #[must_use]
    pub fn view(&self) -> View {
        View {
            title: self.title(),
            left: PaneView::from_pane(&self.panes[0], self.active == Side::Left),
            right: PaneView::from_pane(&self.panes[1], self.active == Side::Right),
            log: LogView {
                lines: self.log.lines(),
                scroll_top: self.log.scroll_top(),
            },
            popup: self.popup_view(),
            split_percent: self.split_percent,
            log_rows: self.log.viewport_rows(),
            style: Arc::clone(&self.style),
        }
    }

    #[must_use]
    pub fn pane(&self, side: Side) -> &Pane {
        &self.panes[side.index()]
    }

    #[must_use]
    pub fn active(&self) -> Side {
        self.active
    }

    #[must_use]
    pub fn config(&self) -> &Config {
        &self.config
    }

    #[cfg(test)]
    pub(crate) fn popup(&self) -> Option<&Popup> {
        self.popup.as_ref()
    }

    fn active_pane(&self) -> &Pane {
        &self.panes[self.active.index()]
    }

    fn active_pane_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active.index()]
    }

    fn run_command(&mut self, command: Command, effects: &mut Vec<Effect>) {
        let side = self.active;
        let pane = &mut self.panes[side.index()];
        match command {
            Command::Open => match pane.cursor_entry() {
                None => self.go_parent(effects),
                Some(entry) if entry.kind.is_dir_like() => {
                    let location = Location::new(pane.location().base.join(&entry.name));
                    self.navigate(side, location, None, HistoryMove::Visit, effects);
                }
                Some(entry) if entry.kind.is_broken_link() => {
                    let path = pane.location().base.join(&entry.name);
                    self.log.push(AppError::BrokenLink { path }.to_string());
                }
                Some(_) => {}
            },
            Command::GoParent => self.go_parent(effects),
            Command::GoRoot => self.go_root(effects),
            Command::GoHome => self.go_home(effects),
            Command::HistoryBack => self.history_back(effects),
            Command::HistoryForward => self.history_forward(effects),
            Command::Drives => self.list_drives(DrivePurpose::Show, command, effects),
            Command::DrivePrev => self.list_drives(DrivePurpose::Prev, command, effects),
            Command::DriveNext => self.list_drives(DrivePurpose::Next, command, effects),
            Command::SyncToOther => self.sync_to_other(effects),
            Command::SyncOtherToThis => self.sync_other_to_this(effects),
            Command::SwapPanes => self.swap_panes(effects),
            Command::FocusLeft => self.active = Side::Left,
            Command::FocusRight => self.active = Side::Right,
            Command::SwitchPane => self.active = side.other(),
            Command::CursorUp => pane.cursor_up(),
            Command::CursorDown => pane.cursor_down(),
            Command::PageUp => pane.page_up(),
            Command::PageDown => pane.page_down(),
            Command::CursorTop => pane.cursor_top(),
            Command::CursorBottom => pane.cursor_bottom(),
            Command::Search => self.open_search(),
            Command::Jump(c) => pane.jump_to(c),
            Command::MarkDown => pane.mark_down(),
            Command::MarkUp => pane.mark_up(),
            Command::MarkRangeAbove => pane.mark_range_above(),
            Command::MarkRangeBelow => pane.mark_range_below(),
            Command::InvertMarks => pane.invert_marks(false),
            Command::InvertMarksAll => pane.invert_marks(true),
            Command::MarkGlob => self.open_input(InputKind::MarkGlob),
            Command::NextMark => pane.next_mark(),
            Command::PrevMark => pane.prev_mark(),
            Command::ClearMarks => pane.clear_marks(),
            Command::Sort => self.open_sort(),
            Command::ToggleHidden => pane.toggle_hidden(),
            Command::Filter => self.open_input(InputKind::Filter),
            Command::ClearFilter => {
                pane.set_filter(None);
            }
            Command::Reload => self.refresh(side, effects),
            Command::PaneShrinkLeft => {
                self.split_percent = (self.split_percent - SPLIT_STEP).max(SPLIT_MIN);
            }
            Command::PaneGrowLeft => {
                self.split_percent = (self.split_percent + SPLIT_STEP).min(SPLIT_MAX);
            }
            Command::LogShrink => {
                let rows = (self.log.viewport_rows() - 1).max(LOG_ROWS_MIN);
                self.log.set_viewport_rows(rows);
            }
            Command::LogGrow => {
                let rows = (self.log.viewport_rows() + 1).min(LOG_ROWS_MAX);
                self.log.set_viewport_rows(rows);
            }
            Command::LogScrollUp => self.log.scroll_up(),
            Command::LogScrollDown => self.log.scroll_down(),
            Command::LogPageUp => self.log.page_up(),
            Command::LogPageDown => self.log.page_down(),
            Command::ClearLog => self.log.clear(),
            Command::Palette => self.open_palette(),
            Command::GoToPath => self.open_input(InputKind::GoToPath),
            Command::Bookmarks => self.open_bookmarks(),
            Command::History => self.open_history(),
            Command::EditConfig => self.edit_config(effects),
            Command::NewWindow => self.new_window(effects),
            Command::Quit => self.quit(true, effects),
            Command::QuitWithoutSaving => self.quit(false, effects),
            Command::External(name) => self.run_external(&name, effects),
            Command::OpenWithOs
            | Command::Copy
            | Command::Move
            | Command::Delete
            | Command::Trash
            | Command::Rename
            | Command::Mkdir
            | Command::NewFile
            | Command::NewText
            | Command::Edit
            | Command::CopyPath
            | Command::CopyName
            | Command::ShellMenu
            | Command::EnterAsArchive
            | Command::Extract
            | Command::Compress
            | Command::Nop => {}
        }
    }

    fn mouse(&mut self, input: MouseInput, effects: &mut Vec<Effect>) {
        match input {
            MouseInput::Click { side, row } | MouseInput::RightClick { side, row } => {
                self.active = side;
                self.panes[side.index()].set_cursor(row);
            }
            MouseInput::DoubleClick { side, row } => {
                self.active = side;
                self.panes[side.index()].set_cursor(row);
                self.run_command(Command::Open, effects);
            }
            MouseInput::Wheel { side, rows } => self.panes[side.index()].scroll_by(rows),
            MouseInput::LogWheel { rows } => self.log.scroll_by(i64::from(rows)),
        }
    }

    fn title(&self) -> String {
        let path = self.active_pane().location().to_string();
        if path.is_empty() {
            "wafu".to_owned()
        } else {
            format!("wafu - {path}")
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
