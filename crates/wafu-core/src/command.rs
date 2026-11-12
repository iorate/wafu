use std::fmt;

use crate::app::Os;

const NAMES: &[(Command, &str)] = &[
    (Command::Open, "open"),
    (Command::OpenWithOs, "open_with_os"),
    (Command::OpenAsText, "open_as_text"),
    (Command::GoParent, "go_parent"),
    (Command::GoRoot, "go_root"),
    (Command::GoHome, "go_home"),
    (Command::GoToPath, "go_to_path"),
    (Command::Bookmarks, "bookmarks"),
    (Command::History, "history"),
    (Command::HistoryBack, "history_back"),
    (Command::HistoryForward, "history_forward"),
    (Command::Drives, "drives"),
    (Command::DrivePrev, "drive_prev"),
    (Command::DriveNext, "drive_next"),
    (Command::FocusLeft, "focus_left"),
    (Command::FocusRight, "focus_right"),
    (Command::SwitchPane, "switch_pane"),
    (Command::SyncToOther, "sync_to_other"),
    (Command::SyncOtherToThis, "sync_other_to_this"),
    (Command::SwapPanes, "swap_panes"),
    (Command::CursorUp, "cursor_up"),
    (Command::CursorDown, "cursor_down"),
    (Command::PageUp, "page_up"),
    (Command::PageDown, "page_down"),
    (Command::CursorTop, "cursor_top"),
    (Command::CursorBottom, "cursor_bottom"),
    (Command::Search, "search"),
    (Command::MarkDown, "mark_down"),
    (Command::MarkUp, "mark_up"),
    (Command::MarkRangeAbove, "mark_range_above"),
    (Command::MarkRangeBelow, "mark_range_below"),
    (Command::InvertMarks, "invert_marks"),
    (Command::InvertMarksAll, "invert_marks_all"),
    (Command::MarkGlob, "mark_glob"),
    (Command::NextMark, "next_mark"),
    (Command::PrevMark, "prev_mark"),
    (Command::ClearMarks, "clear_marks"),
    (Command::Sort, "sort"),
    (Command::ToggleHidden, "toggle_hidden"),
    (Command::Filter, "filter"),
    (Command::ClearFilter, "clear_filter"),
    (Command::Reload, "reload"),
    (Command::PaneShrinkLeft, "pane_shrink_left"),
    (Command::PaneGrowLeft, "pane_grow_left"),
    (Command::LogShrink, "log_shrink"),
    (Command::LogGrow, "log_grow"),
    (Command::LogScrollUp, "log_scroll_up"),
    (Command::LogScrollDown, "log_scroll_down"),
    (Command::LogPageUp, "log_page_up"),
    (Command::LogPageDown, "log_page_down"),
    (Command::ClearLog, "clear_log"),
    (Command::CopyLog, "copy_log"),
    (Command::PreviewScrollUp, "preview_scroll_up"),
    (Command::PreviewScrollDown, "preview_scroll_down"),
    (Command::PreviewPageUp, "preview_page_up"),
    (Command::PreviewPageDown, "preview_page_down"),
    (Command::PreviewTop, "preview_top"),
    (Command::PreviewBottom, "preview_bottom"),
    (Command::PreviewToggleMode, "preview_toggle_mode"),
    (Command::PreviewEdit, "preview_edit"),
    (Command::ClosePreview, "close_preview"),
    (Command::Copy, "copy"),
    (Command::Move, "move"),
    (Command::Delete, "delete"),
    (Command::Trash, "trash"),
    (Command::Rename, "rename"),
    (Command::Mkdir, "mkdir"),
    (Command::NewFile, "new_file"),
    (Command::NewText, "new_text"),
    (Command::Edit, "edit"),
    (Command::RunFile, "run_file"),
    (Command::RunCommandLine, "run_command"),
    (Command::CopyPath, "copy_path"),
    (Command::CopyName, "copy_name"),
    (Command::ShellMenu, "shell_menu"),
    (Command::EnterAsArchive, "enter_as_archive"),
    (Command::Extract, "extract"),
    (Command::Compress, "compress"),
    (Command::CalculateSize, "calculate_size"),
    (Command::CalculateSizeOnDisk, "calculate_size_on_disk"),
    (Command::Palette, "palette"),
    (Command::EditConfig, "edit_config"),
    (Command::NewWindow, "new_window"),
    (Command::NewWindowAsAdmin, "new_window_as_admin"),
    (Command::Quit, "quit"),
    (Command::QuitWithoutSaving, "quit_without_saving"),
    (Command::Nop, "nop"),
];

/// A named operation. Keys, the mouse, and the palette all invoke these.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Command {
    Open,
    OpenWithOs,
    /// Opens the cursor file in the text preview, whatever its name says it is.
    OpenAsText,
    GoParent,
    GoRoot,
    GoHome,
    GoToPath,
    Bookmarks,
    History,
    HistoryBack,
    HistoryForward,
    Drives,
    DrivePrev,
    DriveNext,
    FocusLeft,
    FocusRight,
    SwitchPane,
    SyncToOther,
    SyncOtherToThis,
    SwapPanes,
    CursorUp,
    CursorDown,
    PageUp,
    PageDown,
    CursorTop,
    CursorBottom,
    Search,
    /// `jump_a` to `jump_z` and `jump_0` to `jump_9`. The character is lower case.
    Jump(char),
    MarkDown,
    MarkUp,
    MarkRangeAbove,
    MarkRangeBelow,
    InvertMarks,
    InvertMarksAll,
    MarkGlob,
    NextMark,
    PrevMark,
    ClearMarks,
    Sort,
    ToggleHidden,
    Filter,
    ClearFilter,
    Reload,
    PaneShrinkLeft,
    PaneGrowLeft,
    LogShrink,
    LogGrow,
    LogScrollUp,
    LogScrollDown,
    LogPageUp,
    LogPageDown,
    ClearLog,
    CopyLog,
    PreviewScrollUp,
    PreviewScrollDown,
    PreviewPageUp,
    PreviewPageDown,
    PreviewTop,
    PreviewBottom,
    /// Switches a document preview between text and binary.
    PreviewToggleMode,
    /// Opens the previewed file in the editor, closing the preview first.
    PreviewEdit,
    ClosePreview,
    Copy,
    Move,
    Delete,
    Trash,
    Rename,
    Mkdir,
    NewFile,
    NewText,
    Edit,
    /// Asks for a command line, starting from the cursor item's path.
    RunFile,
    /// Asks for a command line, starting empty.
    RunCommandLine,
    CopyPath,
    CopyName,
    ShellMenu,
    EnterAsArchive,
    Extract,
    Compress,
    /// Logs the number of files and folders and the total size of the
    /// targets, counting what is inside folders.
    CalculateSize,
    /// Logs the number of files and folders and the total space the targets
    /// take up on disk, counting what is inside folders.
    CalculateSizeOnDisk,
    Palette,
    EditConfig,
    NewWindow,
    NewWindowAsAdmin,
    Quit,
    QuitWithoutSaving,
    Nop,
    /// A command defined in the configuration, named `$name` in key bindings.
    External(String),
}

/// Where a key press is interpreted. The text preview is one context in its
/// text and binary modes alike: they are views of the same document, and a key
/// means the same in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    FileList,
    Image,
    Text,
}

impl Command {
    /// All built-in commands in their canonical order, with the jump commands after `search`.
    pub fn all() -> impl Iterator<Item = Self> {
        let (before, after) = NAMES.split_at(
            NAMES
                .iter()
                .position(|(command, _)| *command == Self::Search)
                .map_or(NAMES.len(), |index| index + 1),
        );
        before
            .iter()
            .map(|(command, _)| command.clone())
            .chain(('a'..='z').chain('0'..='9').map(Self::Jump))
            .chain(after.iter().map(|(command, _)| command.clone()))
    }

    /// Whether the command does something on the OS. The default key map and
    /// the palette leave out the commands that do not; a configuration may
    /// still bind them, and running one then logs an error.
    #[must_use]
    pub fn available_on(&self, os: Os) -> bool {
        os == Os::Windows
            || !matches!(
                self,
                Self::Drives
                    | Self::DrivePrev
                    | Self::DriveNext
                    | Self::ShellMenu
                    | Self::NewWindowAsAdmin
            )
    }

    /// Whether the palette lists the command in the context and a binding
    /// there may run it. The image preview is for looking through images and
    /// picking them, and the text preview fills the window. Availability is
    /// per context, not per state: the log hidden under a preview and the
    /// split under an image filling the window still count. Commands that do
    /// nothing on some OS still count as available, so that a shared
    /// configuration loads without errors everywhere.
    #[must_use]
    pub fn available_in(&self, context: Context) -> bool {
        match self.target() {
            Target::Log | Target::App => true,
            Target::AnyPreview => context != Context::FileList,
            Target::TextPreview => context == Context::Text,
            Target::Split | Target::Cursor => context != Context::Text,
            Target::ListView | Target::Location | Target::Files => context == Context::FileList,
        }
    }

    /// The command for a name as written in the configuration. `$name` is an
    /// external command; whether it is defined is checked elsewhere.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if let Some(external) = name.strip_prefix('$') {
            return (!external.is_empty()).then(|| Self::External(external.to_owned()));
        }
        if let Some(rest) = name.strip_prefix("jump_") {
            let mut chars = rest.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_lowercase() || c.is_ascii_digit() => {
                    Some(Self::Jump(c))
                }
                _ => None,
            };
        }
        NAMES
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(command, _)| command.clone())
    }

    // Every variant is listed, so that a new command has to choose its target.
    fn target(&self) -> Target {
        match self {
            Self::ClosePreview => Target::AnyPreview,
            Self::PreviewScrollUp
            | Self::PreviewScrollDown
            | Self::PreviewPageUp
            | Self::PreviewPageDown
            | Self::PreviewTop
            | Self::PreviewBottom
            | Self::PreviewToggleMode
            | Self::PreviewEdit => Target::TextPreview,
            Self::LogShrink
            | Self::LogGrow
            | Self::LogScrollUp
            | Self::LogScrollDown
            | Self::LogPageUp
            | Self::LogPageDown
            | Self::ClearLog
            | Self::CopyLog => Target::Log,
            Self::Palette
            | Self::NewWindow
            | Self::NewWindowAsAdmin
            | Self::Quit
            | Self::QuitWithoutSaving
            | Self::EditConfig => Target::App,
            Self::PaneShrinkLeft | Self::PaneGrowLeft => Target::Split,
            Self::CursorUp
            | Self::CursorDown
            | Self::PageUp
            | Self::PageDown
            | Self::CursorTop
            | Self::CursorBottom
            | Self::Search
            | Self::Jump(_)
            | Self::MarkDown
            | Self::MarkUp
            | Self::MarkRangeAbove
            | Self::MarkRangeBelow
            | Self::InvertMarks
            | Self::InvertMarksAll
            | Self::MarkGlob
            | Self::NextMark
            | Self::PrevMark
            | Self::ClearMarks => Target::Cursor,
            Self::Sort | Self::ToggleHidden | Self::Filter | Self::ClearFilter | Self::Reload => {
                Target::ListView
            }
            Self::Open
            | Self::OpenAsText
            | Self::GoParent
            | Self::GoRoot
            | Self::GoHome
            | Self::GoToPath
            | Self::Bookmarks
            | Self::History
            | Self::HistoryBack
            | Self::HistoryForward
            | Self::Drives
            | Self::DrivePrev
            | Self::DriveNext
            | Self::FocusLeft
            | Self::FocusRight
            | Self::SwitchPane
            | Self::SyncToOther
            | Self::SyncOtherToThis
            | Self::SwapPanes
            | Self::EnterAsArchive => Target::Location,
            // `nop` only unbinds a key, which the configuration allows in
            // every context; in the palette it stays with the file list.
            Self::Copy
            | Self::Move
            | Self::Delete
            | Self::Trash
            | Self::Rename
            | Self::Mkdir
            | Self::NewFile
            | Self::NewText
            | Self::Edit
            | Self::RunFile
            | Self::RunCommandLine
            | Self::Compress
            | Self::Extract
            | Self::CalculateSize
            | Self::CalculateSizeOnDisk
            | Self::ShellMenu
            | Self::OpenWithOs
            | Self::CopyPath
            | Self::CopyName
            | Self::External(_)
            | Self::Nop => Target::Files,
        }
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Jump(c) => write!(f, "jump_{c}"),
            Self::External(name) => write!(f, "${name}"),
            _ => {
                let name = NAMES
                    .iter()
                    .find(|(command, _)| command == self)
                    .map_or("?", |(_, name)| name);
                f.write_str(name)
            }
        }
    }
}

enum Target {
    AnyPreview,
    TextPreview,
    Log,
    App,
    Split,
    Cursor,
    ListView,
    Location,
    Files,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_for_every_command() {
        for command in Command::all() {
            let name = command.to_string();
            assert_ne!(name, "?", "{command:?} has no name");
            assert_eq!(Command::from_name(&name), Some(command), "{name}");
        }
    }

    #[test]
    fn all_lists_every_command_once() {
        let commands: Vec<Command> = Command::all().collect();
        let mut unique = commands.clone();
        unique.sort_by_key(ToString::to_string);
        unique.dedup();

        assert_eq!(commands.len(), NAMES.len() + 36);
        assert_eq!(unique.len(), commands.len());
    }

    #[test]
    fn each_target_decides_the_contexts() {
        let cases = [
            (Command::ClosePreview, &[Context::Image, Context::Text][..]),
            (Command::PreviewScrollDown, &[Context::Text]),
            (
                Command::ClearLog,
                &[Context::FileList, Context::Image, Context::Text],
            ),
            (
                Command::Quit,
                &[Context::FileList, Context::Image, Context::Text],
            ),
            (Command::PaneGrowLeft, &[Context::FileList, Context::Image]),
            (Command::Jump('a'), &[Context::FileList, Context::Image]),
            (Command::Sort, &[Context::FileList]),
            (Command::Open, &[Context::FileList]),
            (Command::External("x".to_owned()), &[Context::FileList]),
        ];

        for (command, expected) in cases {
            let actual: Vec<Context> = [Context::FileList, Context::Image, Context::Text]
                .into_iter()
                .filter(|&context| command.available_in(context))
                .collect();
            assert_eq!(actual, expected, "{command}");
        }
    }

    #[test]
    fn jump_names_need_one_lower_case_letter_or_digit() {
        assert_eq!(Command::from_name("jump_"), None);
        assert_eq!(Command::from_name("jump_ab"), None);
        assert_eq!(Command::from_name("jump_A"), None);
        assert_eq!(Command::from_name("jump_-"), None);
        assert_eq!(Command::from_name("jump_é"), None);
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert_eq!(Command::from_name(""), None);
        assert_eq!(Command::from_name("Open"), None);
        assert_eq!(Command::from_name("$"), None);
    }
}
