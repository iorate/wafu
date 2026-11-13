use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use encoding_rs::Encoding;

use crate::command::Command;
use crate::config::Config;
use crate::entry::Directory;
use crate::error::{AppError, IoError};
use crate::key::{Key, KeyBase, KeyInput, NamedKey};
use crate::location::{ArchivePath, Location};
use crate::log::Log;
use crate::operation::{OperationEvent, OperationSpec, Reply};
use crate::pane::{DrivePurpose, Pane, RequestId};
use crate::popup::{InputKind, Popup};
use crate::preview::{Document, ImageData};
use crate::state::{StateError, StateFile};
use crate::text::show_line_breaks;
use crate::view::{LogView, PaneView, Style, View, status_line};

mod navigation;
mod operations;
mod popups;
mod preview;
mod session;
#[cfg(test)]
mod tests;

const SPLIT_STEP: u8 = 5;
const SPLIT_MIN: u8 = 10;
const SPLIT_MAX: u8 = 90;
const LOG_ROWS_DEFAULT: usize = 6;
const LOG_ROWS_MIN: usize = 1;
const LOG_ROWS_MAX: usize = 30;
const POPUP_ROWS_DEFAULT: usize = 12;

/// An event delivered to the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// The application has started.
    Start(Box<Start>),
    /// The window's close button, or the Close Window item of the macOS menu.
    CloseRequested,
    Key(KeyInput),
    Mouse(MouseInput),
    /// A field of the popup was edited. Fields are numbered from the top.
    InputChanged {
        field: usize,
        text: String,
    },
    /// The number of rows that fit in a pane's list.
    ViewportRows {
        side: Side,
        rows: usize,
    },
    /// The number of unwrapped lines that fit in the preview.
    PreviewRows {
        rows: usize,
    },
    /// The number of rows that fit in the popup's list.
    PopupRows {
        rows: usize,
    },
    Io(IoEvent),
}

/// A pointer action. A `row` is the row under the pointer, counted from the
/// first row of its list rather than the first visible one, and may be past
/// the last row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseInput {
    /// A click on a pane.
    Click { side: Side, row: usize },
    /// A double click on a pane.
    DoubleClick { side: Side, row: usize },
    /// A right click on a pane, sent when the button is released.
    RightClick { side: Side, row: usize },
    /// Wheel movement over a pane, in rows; positive scrolls down.
    Wheel { side: Side, rows: i32 },
    /// The scrollbar was dragged to put `top` at the first visible row. It
    /// may be past the last position and is clamped.
    ScrollTo { side: Side, top: usize },
    /// Wheel movement over the log, in rows.
    LogWheel { rows: i32 },
    /// The log's scrollbar was dragged; same as `ScrollTo`.
    LogScrollTo { top: usize },
    /// The divider between the panes was dragged to put the left pane at
    /// `percent` of the width. It is rounded to the step of the keys and
    /// clamped.
    SplitTo { percent: u8 },
    /// The divider above the log was dragged to show `rows` lines. It is
    /// clamped.
    LogRowsTo { rows: usize },
    /// Wheel movement over a document preview, in rows.
    PreviewWheel { rows: i32 },
    /// A double click on a preview of any kind.
    PreviewDoubleClick,
    /// A click on the popup's list. Separators count as rows.
    PopupClick { row: usize },
    /// A double click on the popup's list.
    PopupDoubleClick { row: usize },
    /// Wheel movement over the popup's list, in rows.
    PopupWheel { rows: i32 },
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
    /// Whether the process runs with administrator rights (Windows).
    pub elevated: bool,
    pub config_path: PathBuf,
    pub config: ConfigSource,
    /// `Ok(None)` when there is no state file.
    pub state: Result<Option<StateFile>, StateError>,
    /// The file `state` was read from: the state file, or the file handed
    /// over by the instance that started this one.
    pub state_read_from: PathBuf,
    /// Whether another instance started this one and handed over its state.
    pub inherited: bool,
}

/// How the configuration file was found at start-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSource {
    /// The file and the files it extends, as for `IoEvent::ConfigLoaded`.
    Read(Vec<ConfigFile>),
    /// There was none, so the template was written.
    Created,
}

/// One file of the configuration as read. A file reached through `extends`
/// more than once appears once for each time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub content: ConfigContent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigContent {
    Text(String),
    Unreadable(IoError),
    /// The file was reached again through `extends` while the files it
    /// extends were still being read, so it was not read again.
    Cycle,
}

/// A request from the application to the outside world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ReadDir {
        side: Side,
        location: Location,
        request: RequestId,
    },
    /// Finds an existing directory at the path, or, when `climb` is set, at
    /// the nearest ancestor that exists.
    Resolve {
        side: Side,
        path: PathBuf,
        climb: bool,
        request: RequestId,
    },
    /// Cancels any request on the reading thread: a `ReadDir`, `Resolve`,
    /// `LoadPreview`, or `LoadImage`.
    CancelRead {
        request: RequestId,
    },
    /// Reads the start of a file and decodes it for the preview.
    LoadPreview {
        source: PreviewSource,
        request: RequestId,
    },
    /// Decodes an image file for the preview.
    LoadImage {
        source: PreviewSource,
        request: RequestId,
    },
    /// Lets go of the archive kept open for previewing its entries, once the
    /// preview closes or the pane leaves the archive.
    ReleasePreview,
    /// Watches a directory for changes, replacing the side's previous watch.
    Watch {
        side: Side,
        path: PathBuf,
    },
    /// Lists the drive letters (Windows only).
    ListDrives {
        request: RequestId,
    },
    /// Watches the configuration file and the files it extends, and reads
    /// them all again on every change to any of them.
    WatchConfig(PathBuf),
    /// Watches the color scheme of the OS and reports it, now and on every
    /// change. Only Windows reports anything; elsewhere the window follows
    /// the OS by itself.
    WatchColorScheme,
    /// Reads the environment of the user's login shell for the programs
    /// started from now on, and reports through `IoEvent::ShellEnvLoaded`.
    /// Asked for at start-up on macOS and Linux, where apps started by a
    /// desktop launcher lack what the shell's rc files set up, unless the
    /// configuration turns it off.
    LoadShellEnv,
    /// Starts a program without a shell and without waiting for it. `name`
    /// is what the log calls it.
    Spawn {
        name: SpawnName,
        program: String,
        args: Vec<OsString>,
        cwd: PathBuf,
    },
    /// Writes the state file. The window geometry is filled in by the UI,
    /// which is the only layer that knows it.
    SaveState(StateFile),
    /// Starts another instance that takes over the state, elevated through
    /// UAC when `as_admin` is set (Windows only). The window geometry is
    /// filled in by the UI, as for `SaveState`.
    SpawnNewWindow {
        state: StateFile,
        as_admin: bool,
    },
    Quit,
    /// Runs a file operation on a worker; it reports through `IoEvent::Operation`.
    StartOperation(OperationSpec),
    /// Answers the running operation.
    OperationReply(Reply),
    Rename {
        from: PathBuf,
        to: PathBuf,
    },
    /// Creates the directory and any missing parents.
    Mkdir(PathBuf),
    /// Creates an empty file; an existing one is an error.
    CreateFile(PathBuf),
    /// Puts text on the clipboard.
    Clipboard(String),
    /// Opens the path with whatever the OS associates with it.
    OpenWithOs(PathBuf),
    /// Shows the OS's context menu for the paths (Windows only).
    ShellMenu {
        paths: Vec<PathBuf>,
        anchor: MenuAnchor,
    },
}

/// Where a context menu opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuAnchor {
    /// At the pointer, for a right click.
    Pointer,
    /// Below a display row of a pane, for the keyboard.
    Row { side: Side, row: usize },
}

/// What to read for a preview: a file, or one entry of an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewSource {
    /// The file, or the archive holding the entry.
    pub path: PathBuf,
    /// The directory inside the archive and the entry's name in it.
    pub entry: Option<(ArchivePath, String)>,
    /// The password the pane knows, for an encrypted entry.
    pub password: Option<String>,
    /// The encoding the user chose for the text; `None` detects it.
    pub encoding: Option<&'static Encoding>,
}

/// Why a preview could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviewError {
    /// The entry is encrypted. `retry` means the password given was wrong.
    PasswordRequired {
        retry: bool,
    },
    Failed(IoError),
}

/// What `Effect::Rename`, `Effect::Mkdir`, or `Effect::CreateFile` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Renamed { from: PathBuf, to: PathBuf },
    DirCreated(PathBuf),
    FileCreated(PathBuf),
}

/// What `Effect::Spawn` or `Effect::SpawnNewWindow` started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnKind {
    External {
        name: SpawnName,
        program: String,
        args: Vec<OsString>,
    },
    NewWindow {
        as_admin: bool,
    },
}

/// What a started program is called in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnName {
    /// The configured editor.
    Editor,
    /// The command that started it.
    Command(Command),
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

/// The color scheme the OS is set to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorScheme {
    Light,
    Dark,
}

/// The result of an effect, reported by the I/O layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoEvent {
    DirLoaded {
        side: Side,
        request: RequestId,
        result: Result<Directory, IoError>,
    },
    /// `None` when nothing exists at the path, nor, when climbing, above it.
    Resolved {
        side: Side,
        request: RequestId,
        result: Option<Resolved>,
    },
    PreviewLoaded {
        request: RequestId,
        result: Result<Document, PreviewError>,
    },
    ImageLoaded {
        request: RequestId,
        result: Result<ImageData, PreviewError>,
    },
    /// Something changed in the watched directory.
    LocationChanged {
        side: Side,
        path: PathBuf,
    },
    /// Always empty except on Windows.
    Drives {
        request: RequestId,
        letters: Vec<char>,
    },
    WatchFailed {
        side: Side,
        path: PathBuf,
        error: IoError,
    },
    /// A file of the configuration changed, and they were all read again.
    /// The files are in the order they merge: the bases of each file before
    /// it, so that the configuration file itself comes last.
    ConfigLoaded {
        files: Vec<ConfigFile>,
    },
    ConfigWatchFailed {
        path: PathBuf,
        error: IoError,
    },
    /// The color scheme of the OS, when first read and on every change.
    ColorScheme {
        scheme: ColorScheme,
    },
    /// The answer to `Effect::LoadShellEnv`, sent exactly once. The programs
    /// keep the original environment when it is an error.
    ShellEnvLoaded {
        error: Option<IoError>,
    },
    Spawned {
        kind: SpawnKind,
        error: Option<IoError>,
    },
    /// A program started by `Effect::Spawn` ended. `code` is `None` when a
    /// signal ended it.
    Exited {
        name: SpawnName,
        code: Option<i32>,
    },
    Operation(OperationEvent),
    Changed {
        change: Change,
        error: Option<IoError>,
    },
    ClipboardSet {
        text: String,
        error: Option<IoError>,
    },
    Opened {
        path: PathBuf,
        error: Option<IoError>,
    },
    /// The shell menu was closed, or could not be shown.
    ShellMenuClosed {
        error: Option<IoError>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

impl fmt::Display for SpawnName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Editor => f.write_str("editor"),
            Self::Command(command) => write!(f, "{command}"),
        }
    }
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

/// The whole application state.
#[derive(Debug)]
pub struct App {
    panes: [Pane; 2],
    active: Side,
    config: Config,
    config_path: PathBuf,
    // What the OS reported; `None` until it does.
    color_scheme: Option<ColorScheme>,
    style: Arc<Style>,
    log: Log,
    popup: Option<Popup>,
    popup_top: usize,
    popup_rows: usize,
    popup_generation: usize,
    preview: Option<preview::Preview>,
    // What the UI last reported; kept across previews so that a new one
    // starts with a sensible page size.
    preview_rows: usize,
    operation: Option<operations::Operation>,
    // A file made for `new_text`, to open in the editor once it exists.
    edit_after_create: Option<(PathBuf, PathBuf)>,
    // Programs to start once the login shell's environment is known, so that
    // none of them starts with the environment wafu was launched with; `None`
    // when nothing is waiting for it.
    held_spawns: Option<Vec<Effect>>,
    split_percent: u8,
    next_request: u64,
    home: PathBuf,
    os: Os,
    elevated: bool,
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
            color_scheme: None,
            style: Arc::new(Style::from_config(&config, Os::Linux, None)),
            config,
            config_path: PathBuf::new(),
            log,
            popup: None,
            popup_top: 0,
            popup_rows: POPUP_ROWS_DEFAULT,
            popup_generation: 0,
            preview: None,
            preview_rows: 1,
            operation: None,
            edit_after_create: None,
            held_spawns: None,
            split_percent: 50,
            next_request: 1,
            home: PathBuf::new(),
            os: Os::Linux,
            elevated: false,
        }
    }

    /// Applies a message and returns the effects it produced.
    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        let popup_before = self.popup_shape();
        let archive_before = self.previewed_archive();
        let mut effects = Vec::new();
        match msg {
            Msg::Start(start) => {
                self.start(*start, &mut effects);
                return effects;
            }
            Msg::CloseRequested => {
                if self.operation.is_some() {
                    self.confirm_quit();
                } else {
                    self.quit(true, &mut effects);
                }
            }
            Msg::Key(input) => {
                if let Some(key) = Key::from_input(&input) {
                    if self.operation.is_some() {
                        self.operation_key(key, &mut effects);
                    } else if self.popup.is_some() {
                        self.popup_key(key, &mut effects);
                    } else if self.navigating() {
                        // A move in progress is modal: only Esc, which cancels it, gets through.
                        if key.base == KeyBase::Named(NamedKey::Escape) {
                            self.cancel_navigation(&mut effects);
                        }
                    } else {
                        if let Some(command) = self.config.keys.lookup(self.context(), &key) {
                            self.run_command(command, &mut effects);
                        }
                        // A key after scrolling with the mouse brings the cursor back into view.
                        self.active_pane_mut().ensure_cursor_visible();
                    }
                }
            }
            Msg::Mouse(input) => {
                // Unlike a key, a click goes to the popup before the operation,
                // since a conflict or quit confirmation opens during one.
                if self.popup.is_some() {
                    self.popup_mouse(input, &mut effects);
                } else if self.operation.is_some() || self.navigating() {
                } else if self.preview.is_some() {
                    self.preview_mouse(input, &mut effects);
                } else {
                    self.mouse(input, &mut effects);
                }
            }
            Msg::InputChanged { field, text } => self.input_changed(field, text),
            Msg::ViewportRows { side, rows } => self.panes[side.index()].set_viewport_rows(rows),
            Msg::PreviewRows { rows } => self.preview_rows = rows.max(1),
            Msg::PopupRows { rows } => self.set_popup_rows(rows),
            Msg::Io(event) => self.io_event(event, &mut effects),
        }
        // Whatever moved the cursor, the image preview follows it.
        self.sync_image_preview(&mut effects);
        self.release_preview(archive_before.as_deref(), &mut effects);
        self.follow_popup(popup_before);
        self.hold_spawns(&mut effects);
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
            status: status_line(self.active_pane()),
            popup: self.popup_view(),
            preview: self.preview_view(),
            hide_log: self.hide_log(),
            split_percent: self.split_percent,
            log_rows: self.log.viewport_rows(),
            style: Arc::clone(&self.style),
        }
    }

    #[must_use]
    pub fn active(&self) -> Side {
        self.active
    }

    #[must_use]
    pub(crate) fn pane(&self, side: Side) -> &Pane {
        &self.panes[side.index()]
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

    // One arm for each command, so the length follows the number of commands.
    #[allow(clippy::too_many_lines)]
    fn run_command(&mut self, command: Command, effects: &mut Vec<Effect>) {
        let side = self.active;
        let pane = &mut self.panes[side.index()];
        match command {
            Command::Open => self.open_entry(effects),
            Command::OpenAsText => self.open_as_text(effects),
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
            Command::Reload => self.reload(side, effects),
            Command::PaneShrinkLeft => self.split_to(self.split_percent - SPLIT_STEP),
            Command::PaneGrowLeft => self.split_to(self.split_percent + SPLIT_STEP),
            Command::LogShrink => self.log_rows_to(self.log.viewport_rows().saturating_sub(1)),
            Command::LogGrow => self.log_rows_to(self.log.viewport_rows() + 1),
            Command::LogScrollUp => self.log.scroll_up(),
            Command::LogScrollDown => self.log.scroll_down(),
            Command::LogPageUp => self.log.page_up(),
            Command::LogPageDown => self.log.page_down(),
            Command::ClearLog => self.log.clear(),
            Command::CopyLog => self.copy_log(effects),
            Command::PreviewScrollUp
            | Command::PreviewScrollDown
            | Command::PreviewPageUp
            | Command::PreviewPageDown
            | Command::PreviewTop
            | Command::PreviewBottom
            | Command::PreviewToggleMode
            | Command::PreviewEncoding
            | Command::PreviewEdit
            | Command::ClosePreview => self.run_preview_command(&command, effects),
            Command::Palette => self.open_palette(),
            Command::GoToPath => self.open_input(InputKind::GoToPath {
                dir: self.active_pane().location().base.clone(),
            }),
            Command::Bookmarks => self.open_bookmarks(),
            Command::History => self.open_history(),
            Command::EditConfig => self.edit_config(effects),
            Command::NewWindow => self.new_window(false, effects),
            Command::NewWindowAsAdmin if self.os != Os::Windows => {
                self.log.push(AppError::WindowsOnly { command }.to_string());
            }
            Command::NewWindowAsAdmin => self.new_window(true, effects),
            Command::Quit => self.quit(true, effects),
            Command::QuitWithoutSaving => self.quit(false, effects),
            Command::RunFile => self.run_file(effects),
            Command::RunCommandLine => self.open_input(InputKind::RunCommand),
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
            | Command::CalculateSize
            | Command::CalculateSizeOnDisk => self.run_file_command(command, effects),
            Command::Nop => {}
        }
    }

    fn mouse(&mut self, input: MouseInput, effects: &mut Vec<Effect>) {
        match input {
            MouseInput::Click { side, row } => {
                self.active = side;
                self.panes[side.index()].set_cursor(row);
            }
            MouseInput::RightClick { side, row } => {
                self.active = side;
                let pane = &mut self.panes[side.index()];
                if row < pane.row_count() {
                    pane.set_cursor(row);
                    if self.os == Os::Windows {
                        self.shell_menu(MenuAnchor::Pointer, effects);
                    }
                }
            }
            MouseInput::DoubleClick { side, row } => {
                self.active = side;
                let pane = &mut self.panes[side.index()];
                if row < pane.row_count() {
                    pane.set_cursor(row);
                    self.run_command(Command::Open, effects);
                }
            }
            MouseInput::Wheel { side, rows } => self.panes[side.index()].scroll_by(rows),
            MouseInput::ScrollTo { side, top } => self.panes[side.index()].scroll_to(top),
            MouseInput::LogWheel { rows } => self.log.scroll_by(i64::from(rows)),
            MouseInput::LogScrollTo { top } => self.log.scroll_to(top),
            MouseInput::SplitTo { percent } => self.split_to(percent),
            MouseInput::LogRowsTo { rows } => self.log_rows_to(rows),
            MouseInput::PreviewWheel { .. }
            | MouseInput::PreviewDoubleClick
            | MouseInput::PopupClick { .. }
            | MouseInput::PopupDoubleClick { .. }
            | MouseInput::PopupWheel { .. } => {}
        }
    }

    fn split_to(&mut self, percent: u8) {
        let rounded = (percent.min(100) + SPLIT_STEP / 2) / SPLIT_STEP * SPLIT_STEP;
        self.split_percent = rounded.clamp(SPLIT_MIN, SPLIT_MAX);
    }

    fn log_rows_to(&mut self, rows: usize) {
        self.log
            .set_viewport_rows(rows.clamp(LOG_ROWS_MIN, LOG_ROWS_MAX));
    }

    fn copy_log(&self, effects: &mut Vec<Effect>) {
        let lines = self.log.lines();
        if !lines.is_empty() {
            effects.push(Effect::Clipboard(lines.join("\n")));
        }
    }

    fn io_event(&mut self, event: IoEvent, effects: &mut Vec<Effect>) {
        match event {
            IoEvent::DirLoaded {
                side,
                request,
                result,
            } => self.dir_loaded(side, request, result, effects),
            IoEvent::Resolved {
                side,
                request,
                result,
            } => self.resolved(side, request, result, effects),
            IoEvent::PreviewLoaded { request, result } => {
                self.preview_loaded(request, result, effects);
            }
            IoEvent::ImageLoaded { request, result } => self.image_loaded(request, result),
            IoEvent::LocationChanged { side, path } => {
                self.location_changed(side, &path, effects);
            }
            IoEvent::Drives { request, letters } => self.drives(request, letters, effects),
            IoEvent::WatchFailed { path, error, .. }
            | IoEvent::ConfigWatchFailed { path, error } => {
                self.log.push(
                    AppError::Watch {
                        path,
                        source: error,
                    }
                    .to_string(),
                );
            }
            IoEvent::ConfigLoaded { files } => self.config_loaded(files),
            IoEvent::ColorScheme { scheme } => self.color_scheme_changed(scheme),
            IoEvent::ShellEnvLoaded { error } => self.shell_env_loaded(error, effects),
            IoEvent::Spawned { kind, error } => self.spawned(kind, error),
            IoEvent::Exited { name, code } => self.exited(name, code),
            IoEvent::Operation(event) => self.operation_event(event, effects),
            IoEvent::Changed { change, error } => {
                self.changed(change, error, effects);
            }
            IoEvent::ClipboardSet { text, error } => {
                if let Some(source) = error {
                    self.log
                        .push(AppError::ClipboardFailed { source }.to_string());
                } else {
                    let lines = text.lines().count();
                    self.log.push(if lines == 1 {
                        format!("Copied to clipboard: {text}")
                    } else {
                        format!("Copied {lines} lines to clipboard")
                    });
                }
            }
            IoEvent::Opened { path, error } => {
                if let Some(source) = error {
                    self.log
                        .push(AppError::OpenFailed { path, source }.to_string());
                }
            }
            IoEvent::ShellMenuClosed { error } => {
                if let Some(source) = error {
                    self.log
                        .push(AppError::ShellMenuFailed { source }.to_string());
                }
            }
        }
    }

    fn title(&self) -> String {
        let name = if self.elevated {
            "wafu (Administrator)"
        } else {
            "wafu"
        };
        let path = show_line_breaks(self.active_pane().location().to_string());
        if path.is_empty() {
            name.to_owned()
        } else {
            format!("{name} - {path}")
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}
