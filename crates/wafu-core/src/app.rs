use std::ffi::OsString;
use std::path::PathBuf;

use crate::command::{Command, Context};
use crate::entry::Entry;
use crate::error::{AppError, IoError};
use crate::key::{Key, KeyInput};
use crate::keymap::KeyMap;
use crate::location::Location;
use crate::log::Log;
use crate::pane::{Pane, RequestId};
use crate::popup::{InputKind, Popup};
use crate::view::{Ellipsis, LogView, PaneView, View};

mod popups;
#[cfg(test)]
mod tests;

const SPLIT_STEP: u8 = 5;
const SPLIT_MIN: u8 = 10;
const SPLIT_MAX: u8 = 90;
const LOG_ROWS_MIN: usize = 1;
const LOG_ROWS_MAX: usize = 30;

/// An event delivered to the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// The application has started with the given initial pane locations.
    Start {
        left: PathBuf,
        right: PathBuf,
    },
    Key(KeyInput),
    Mouse(MouseInput),
    /// The text of the popup's input field was edited.
    InputChanged(String),
    /// The number of rows that fit in a pane's list.
    ViewportRows {
        side: Side,
        rows: usize,
    },
    /// The number of rows that fit in the log.
    LogViewportRows(usize),
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
}

/// A request from the application to the outside world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ReadDir {
        side: Side,
        location: Location,
        request: RequestId,
    },
    CancelRead {
        request: RequestId,
    },
    /// Watches a directory for changes, replacing the side's previous watch.
    Watch {
        side: Side,
        path: PathBuf,
    },
    SetTitle(String),
}

/// The result of an effect, reported by the I/O layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoEvent {
    DirLoaded {
        side: Side,
        request: RequestId,
        result: Result<Vec<Entry>, IoError>,
    },
    /// Something changed in the watched directory.
    LocationChanged { side: Side, path: PathBuf },
    WatchFailed {
        side: Side,
        path: PathBuf,
        error: IoError,
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
    keymap: KeyMap,
    log: Log,
    popup: Option<Popup>,
    split_percent: u8,
    log_rows: usize,
    ellipsis: Ellipsis,
    next_request: u64,
}

impl Side {
    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        Self {
            panes: [
                Pane::new(Location::new(PathBuf::new())),
                Pane::new(Location::new(PathBuf::new())),
            ],
            active: Side::Left,
            keymap: KeyMap::default(),
            log: Log::new(),
            popup: None,
            split_percent: 50,
            log_rows: 6,
            ellipsis: Ellipsis::End,
            next_request: 1,
        }
    }

    /// Applies a message and returns the effects it produced.
    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        let title_before = self.title();
        let mut effects = Vec::new();
        match msg {
            Msg::Start { left, right } => {
                self.panes = [
                    Pane::new(Location::new(PathBuf::new())),
                    Pane::new(Location::new(PathBuf::new())),
                ];
                self.navigate(Side::Left, Location::new(left), None, &mut effects);
                self.navigate(Side::Right, Location::new(right), None, &mut effects);
                effects.push(Effect::SetTitle(self.title()));
                return effects;
            }
            Msg::Key(input) => {
                if let Some(key) = Key::from_input(&input) {
                    if self.popup.is_some() {
                        self.popup_key(key, &mut effects);
                    } else {
                        if let Some(command) = self.keymap.lookup(Context::FileList, &key) {
                            self.run_command(command, &mut effects);
                        }
                        // A key after wheel scrolling brings the cursor back into view.
                        self.active_pane_mut().ensure_cursor_visible();
                    }
                }
            }
            Msg::Mouse(input) => {
                if self.popup.is_none() {
                    self.mouse(input, &mut effects);
                }
            }
            Msg::InputChanged(text) => self.input_changed(text),
            Msg::ViewportRows { side, rows } => self.panes[side.index()].set_viewport_rows(rows),
            Msg::LogViewportRows(rows) => self.log.set_viewport_rows(rows),
            Msg::Io(IoEvent::DirLoaded {
                side,
                request,
                result,
            }) => self.dir_loaded(side, request, result, &mut effects),
            Msg::Io(IoEvent::LocationChanged { side, path }) => {
                if self.panes[side.index()].location.base == path {
                    self.refresh(side, &mut effects);
                }
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
                lines: self.log.lines().map(str::to_owned).collect(),
                scroll_top: self.log.scroll_top(),
            },
            popup: self.popup_view(),
            split_percent: self.split_percent,
            log_rows: self.log_rows,
            ellipsis: self.ellipsis,
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
                    let location = Location::new(pane.location.base.join(&entry.name));
                    self.navigate(side, location, None, effects);
                }
                Some(_) => {}
            },
            Command::GoParent => self.go_parent(effects),
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
            Command::LogShrink => self.log_rows = (self.log_rows - 1).max(LOG_ROWS_MIN),
            Command::LogGrow => self.log_rows = (self.log_rows + 1).min(LOG_ROWS_MAX),
            Command::LogScrollUp => self.log.scroll_up(),
            Command::LogScrollDown => self.log.scroll_down(),
            Command::LogPageUp => self.log.page_up(),
            Command::LogPageDown => self.log.page_down(),
            Command::ClearLog => self.log.clear(),
            Command::Palette => self.open_palette(),
            _ => {}
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
        }
    }

    fn go_parent(&mut self, effects: &mut Vec<Effect>) {
        let side = self.active;
        let pane = &self.panes[side.index()];
        if let Some(parent) = pane.location.parent() {
            let came_from = pane.location.base.file_name().map(OsString::from);
            self.navigate(side, parent, came_from, effects);
        }
    }

    fn navigate(
        &mut self,
        side: Side,
        location: Location,
        pending_cursor: Option<OsString>,
        effects: &mut Vec<Effect>,
    ) {
        let pane = &mut self.panes[side.index()];
        if let Some(old) = pane.loading.take() {
            effects.push(Effect::CancelRead { request: old });
        }
        pane.set_location(location.clone(), pending_cursor);
        // Watching before reading means a change during the read is not missed.
        effects.push(Effect::Watch {
            side,
            path: location.base.clone(),
        });
        self.read(side, location, effects);
    }

    fn refresh(&mut self, side: Side, effects: &mut Vec<Effect>) {
        let pane = &mut self.panes[side.index()];
        if pane.loading.is_some() {
            pane.reload_pending = true;
            return;
        }
        let location = pane.location.clone();
        self.read(side, location, effects);
    }

    fn read(&mut self, side: Side, location: Location, effects: &mut Vec<Effect>) {
        let request = RequestId(self.next_request);
        self.next_request += 1;
        self.panes[side.index()].loading = Some(request);
        effects.push(Effect::ReadDir {
            side,
            location,
            request,
        });
    }

    fn dir_loaded(
        &mut self,
        side: Side,
        request: RequestId,
        result: Result<Vec<Entry>, IoError>,
        effects: &mut Vec<Effect>,
    ) {
        let pane = &mut self.panes[side.index()];
        if pane.loading != Some(request) {
            return;
        }
        pane.loading = None;
        match result {
            Ok(entries) => pane.set_entries(entries),
            Err(source) => {
                let error = AppError::ReadDir {
                    path: pane.location.base.clone(),
                    source,
                };
                self.log.push(error.to_string());
            }
        }
        let pane = &mut self.panes[side.index()];
        if pane.reload_pending {
            pane.reload_pending = false;
            self.refresh(side, effects);
        }
    }

    fn title(&self) -> String {
        let path = self.active_pane().location.to_string();
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
