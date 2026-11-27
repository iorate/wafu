use std::fmt;

use crate::app::Os;

const JUMP_DESCRIPTION: &str = "Move the cursor to the next item whose name starts with the command's letter or digit, wrapping around";

const NAMES: &[(Command, &str, &str)] = &[
    (
        Command::Open,
        "open",
        "Open the item under the cursor: go into a directory or archive, run a program, or preview any other file",
    ),
    (
        Command::OpenAlternative,
        "open_alternative",
        "Open the item under the cursor with the program configured for its kind or extension",
    ),
    (
        Command::OpenWithOs,
        "open_with_os",
        "Open the item under the cursor with the OS's default application",
    ),
    (
        Command::OpenAsText,
        "open_as_text",
        "Preview the file under the cursor as text, whatever its type",
    ),
    (Command::GoParent, "go_parent", "Go to the parent directory"),
    (
        Command::GoRoot,
        "go_root",
        "Go to the root directory, or to the root of the archive the active pane is in",
    ),
    (Command::GoHome, "go_home", "Go to the home directory"),
    (Command::GoToPath, "go_to_path", "Go to a typed path"),
    (
        Command::Browse,
        "browse",
        "Go to a directory picked in the OS's directory dialog",
    ),
    (
        Command::Find,
        "find",
        "Find the items below the active pane's directory that match a glob pattern and go to the one picked",
    ),
    (
        Command::Bookmarks,
        "bookmarks",
        "Go to the bookmark picked from the list",
    ),
    (
        Command::History,
        "history",
        "Go to a directory picked from the history of the active or the other pane",
    ),
    (
        Command::GoBack,
        "go_back",
        "Go back in the active pane's history",
    ),
    (
        Command::GoForward,
        "go_forward",
        "Go forward in the active pane's history",
    ),
    (
        Command::Drives,
        "drives",
        "Go to the drive picked from the list",
    ),
    (
        Command::GoPrevDrive,
        "go_prev_drive",
        "Go to the previous drive, wrapping around",
    ),
    (
        Command::GoNextDrive,
        "go_next_drive",
        "Go to the next drive, wrapping around",
    ),
    (
        Command::FocusLeft,
        "focus_left",
        "Make the left pane active",
    ),
    (
        Command::FocusRight,
        "focus_right",
        "Make the right pane active",
    ),
    (
        Command::FocusOther,
        "focus_other",
        "Make the other pane active",
    ),
    (
        Command::SyncToOther,
        "sync_to_other",
        "Show the other pane's directory in the active pane",
    ),
    (
        Command::SyncFromOther,
        "sync_from_other",
        "Show the active pane's directory in the other pane",
    ),
    (
        Command::SwapPanes,
        "swap_panes",
        "Swap the directories of the two panes",
    ),
    (
        Command::CursorUp,
        "cursor_up",
        "Move the cursor up one row, wrapping to the bottom",
    ),
    (
        Command::CursorDown,
        "cursor_down",
        "Move the cursor down one row, wrapping to the top",
    ),
    (
        Command::CursorPageUp,
        "cursor_page_up",
        "Move the cursor up one page",
    ),
    (
        Command::CursorPageDown,
        "cursor_page_down",
        "Move the cursor down one page",
    ),
    (
        Command::CursorTop,
        "cursor_top",
        "Move the cursor to the first row",
    ),
    (
        Command::CursorBottom,
        "cursor_bottom",
        "Move the cursor to the last row",
    ),
    (
        Command::Search,
        "search",
        "Move the cursor to an item by typing part of its name",
    ),
    (
        Command::MarkDown,
        "mark_down",
        "Toggle the mark on the item under the cursor and move the cursor down",
    ),
    (
        Command::MarkUp,
        "mark_up",
        "Toggle the mark on the item under the cursor and move the cursor up",
    ),
    (
        Command::MarkRangeAbove,
        "mark_range_above",
        "Mark the items from the nearest mark above the cursor down to the cursor",
    ),
    (
        Command::MarkRangeBelow,
        "mark_range_below",
        "Mark the items from the cursor down to the nearest mark below it",
    ),
    (
        Command::InvertMarks,
        "invert_marks",
        "Invert the marks on the files, leaving directories as they are",
    ),
    (
        Command::InvertMarksAll,
        "invert_marks_all",
        "Invert the marks on the files and the directories",
    ),
    (
        Command::MarkGlob,
        "mark_glob",
        "Mark the files whose names match a glob pattern",
    ),
    (
        Command::MarkCompare,
        "mark_compare",
        "Mark the items missing from the other pane, or also the newer or different ones",
    ),
    (
        Command::CursorNextMark,
        "cursor_next_mark",
        "Move the cursor to the next marked item, wrapping around",
    ),
    (
        Command::CursorPrevMark,
        "cursor_prev_mark",
        "Move the cursor to the previous marked item, wrapping around",
    ),
    (
        Command::ClearMarks,
        "clear_marks",
        "Unmark every item in the active pane",
    ),
    (
        Command::Sort,
        "sort",
        "Sort the active pane by name, extension, size, or time",
    ),
    (
        Command::ToggleHidden,
        "toggle_hidden",
        "Show or hide hidden items in the active pane",
    ),
    (
        Command::Filter,
        "filter",
        "Show only the files in the active pane whose names match a glob pattern",
    ),
    (
        Command::ClearFilter,
        "clear_filter",
        "Remove the active pane's filter",
    ),
    (
        Command::Reload,
        "reload",
        "Read the active pane's directory again",
    ),
    (
        Command::SplitLeft,
        "split_left",
        "Move the border between the panes to the left",
    ),
    (
        Command::SplitRight,
        "split_right",
        "Move the border between the panes to the right",
    ),
    (
        Command::LogShrink,
        "log_shrink",
        "Make the log one line shorter",
    ),
    (Command::LogGrow, "log_grow", "Make the log one line taller"),
    (
        Command::LogScrollUp,
        "log_scroll_up",
        "Scroll the log up one line",
    ),
    (
        Command::LogScrollDown,
        "log_scroll_down",
        "Scroll the log down one line",
    ),
    (
        Command::LogPageUp,
        "log_page_up",
        "Scroll the log up one page",
    ),
    (
        Command::LogPageDown,
        "log_page_down",
        "Scroll the log down one page",
    ),
    (Command::LogTop, "log_top", "Scroll to the top of the log"),
    (
        Command::LogBottom,
        "log_bottom",
        "Scroll to the bottom of the log",
    ),
    (Command::LogClear, "log_clear", "Clear the log"),
    (
        Command::LogCopy,
        "log_copy",
        "Copy the log to the clipboard",
    ),
    (
        Command::PreviewScrollUp,
        "preview_scroll_up",
        "Scroll the text preview up one line",
    ),
    (
        Command::PreviewScrollDown,
        "preview_scroll_down",
        "Scroll the text preview down one line",
    ),
    (
        Command::PreviewPageUp,
        "preview_page_up",
        "Scroll the text preview up one page",
    ),
    (
        Command::PreviewPageDown,
        "preview_page_down",
        "Scroll the text preview down one page",
    ),
    (
        Command::PreviewTop,
        "preview_top",
        "Scroll to the top of the text preview",
    ),
    (
        Command::PreviewBottom,
        "preview_bottom",
        "Scroll to the bottom of the text preview",
    ),
    (
        Command::PreviewToggleMode,
        "preview_toggle_mode",
        "Switch the text preview between text and a hex dump",
    ),
    (
        Command::PreviewEncodings,
        "preview_encodings",
        "Decode the text preview with the encoding picked from the list",
    ),
    (
        Command::PreviewEdit,
        "preview_edit",
        "Close the text preview and open its file in the editor",
    ),
    (Command::PreviewClose, "preview_close", "Close the preview"),
    (
        Command::Copy,
        "copy",
        "Copy the marked items to the other pane",
    ),
    (
        Command::Move,
        "move",
        "Move the marked items to the other pane",
    ),
    (
        Command::Delete,
        "delete",
        "Delete the marked items permanently",
    ),
    (
        Command::Trash,
        "trash",
        "Move the marked items to the trash",
    ),
    (
        Command::Rename,
        "rename",
        "Rename the item under the cursor",
    ),
    (
        Command::RenameReplace,
        "rename_replace",
        "Rename the marked items or the item under the cursor by find and replace",
    ),
    (
        Command::RenameTemplate,
        "rename_template",
        "Rename the marked items or the item under the cursor from a template",
    ),
    (
        Command::NewDir,
        "new_dir",
        "Create a directory in the active pane and go to it",
    ),
    (
        Command::NewFile,
        "new_file",
        "Create an empty file in the active pane",
    ),
    (
        Command::NewText,
        "new_text",
        "Create a file in the active pane and open it in the editor",
    ),
    (
        Command::Edit,
        "edit",
        "Open the file under the cursor in the editor",
    ),
    (
        Command::Run,
        "run",
        "Run a command line that starts out with the path of the item under the cursor",
    ),
    (
        Command::RunCommandLine,
        "run_command_line",
        "Run a command line that starts out empty",
    ),
    (
        Command::CopyPath,
        "copy_path",
        "Copy the paths of the marked items or the item under the cursor to the clipboard",
    ),
    (
        Command::CopyName,
        "copy_name",
        "Copy the names of the marked items or the item under the cursor to the clipboard",
    ),
    (
        Command::CopyFiles,
        "copy_files",
        "Put the marked items or the item under the cursor on the clipboard as files",
    ),
    (
        Command::PasteFiles,
        "paste_files",
        "Copy the files on the clipboard into the active pane",
    ),
    (
        Command::ShellMenu,
        "shell_menu",
        "Run a shell menu command on the marked items or the item under the cursor",
    ),
    (
        Command::CalculateSize,
        "calculate_size",
        "Log the total size of the marked items or the item under the cursor",
    ),
    (
        Command::CalculateSizeOnDisk,
        "calculate_size_on_disk",
        "Log the disk space taken by the marked items or the item under the cursor",
    ),
    (
        Command::OpenAsArchive,
        "open_as_archive",
        "Open the file under the cursor as an archive, whatever its extension",
    ),
    (
        Command::Extract,
        "extract",
        "Extract the marked archives or the archive under the cursor into the other pane",
    ),
    (
        Command::ExtractHere,
        "extract_here",
        "Extract the marked archives or the archive under the cursor into the active pane",
    ),
    (
        Command::Compress,
        "compress",
        "Compress the marked items or the item under the cursor into an archive in the other pane",
    ),
    (
        Command::CompressHere,
        "compress_here",
        "Compress the marked items or the item under the cursor into an archive in the active pane",
    ),
    (
        Command::Palette,
        "palette",
        "Run the command picked from a list of those available here",
    ),
    (
        Command::EditConfig,
        "edit_config",
        "Open the configuration file in the editor",
    ),
    (
        Command::NewWindow,
        "new_window",
        "Open a new window showing the same directories",
    ),
    (
        Command::NewWindowAsAdmin,
        "new_window_as_admin",
        "Open a new window showing the same directories, as administrator",
    ),
    (
        Command::Quit,
        "quit",
        "Quit, saving the state for the next start",
    ),
    (
        Command::QuitWithoutSaving,
        "quit_without_saving",
        "Quit without saving the state",
    ),
    (
        Command::Nop,
        "nop",
        "Do nothing, which unbinds the key bound to it",
    ),
];

/// A named operation. Keys, the mouse, and the palette all invoke these.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Command {
    Open,
    /// Does what `open` does when the configuration has no program for the item.
    OpenAlternative,
    OpenWithOs,
    OpenAsText,
    GoParent,
    GoRoot,
    GoHome,
    GoToPath,
    Browse,
    Find,
    Bookmarks,
    History,
    GoBack,
    GoForward,
    Drives,
    GoPrevDrive,
    GoNextDrive,
    FocusLeft,
    FocusRight,
    FocusOther,
    SyncToOther,
    SyncFromOther,
    SwapPanes,
    CursorUp,
    CursorDown,
    CursorPageUp,
    CursorPageDown,
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
    MarkCompare,
    CursorNextMark,
    CursorPrevMark,
    ClearMarks,
    Sort,
    ToggleHidden,
    Filter,
    ClearFilter,
    Reload,
    SplitLeft,
    SplitRight,
    LogShrink,
    LogGrow,
    LogScrollUp,
    LogScrollDown,
    LogPageUp,
    LogPageDown,
    LogTop,
    LogBottom,
    LogClear,
    LogCopy,
    PreviewScrollUp,
    PreviewScrollDown,
    PreviewPageUp,
    PreviewPageDown,
    PreviewTop,
    PreviewBottom,
    PreviewToggleMode,
    PreviewEncodings,
    PreviewEdit,
    PreviewClose,
    Copy,
    Move,
    Delete,
    Trash,
    Rename,
    RenameReplace,
    RenameTemplate,
    NewDir,
    NewFile,
    NewText,
    Edit,
    Run,
    RunCommandLine,
    CopyPath,
    CopyName,
    /// Puts the targets on the clipboard as files, for pasting in another
    /// application or with `paste_files`.
    CopyFiles,
    PasteFiles,
    ShellMenu,
    /// Logs the number of files and directories and the total size of the
    /// targets, counting what is inside directories.
    CalculateSize,
    /// Logs the number of files and directories and the total space the
    /// targets take up on disk, counting what is inside directories.
    CalculateSizeOnDisk,
    OpenAsArchive,
    Extract,
    ExtractHere,
    Compress,
    CompressHere,
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
                .position(|(command, _, _)| *command == Self::Search)
                .map_or(NAMES.len(), |index| index + 1),
        );
        before
            .iter()
            .map(|(command, _, _)| command.clone())
            .chain(('a'..='z').chain('0'..='9').map(Self::Jump))
            .chain(after.iter().map(|(command, _, _)| command.clone()))
    }

    /// A one-line description for the palette and the command reference.
    /// The jump commands share one; external commands have none.
    #[must_use]
    pub fn description(&self) -> Option<&'static str> {
        match self {
            Self::Jump(_) => Some(JUMP_DESCRIPTION),
            Self::External(_) => None,
            _ => NAMES
                .iter()
                .find(|(command, _, _)| command == self)
                .map(|(_, _, description)| *description),
        }
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
                    | Self::GoPrevDrive
                    | Self::GoNextDrive
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
            .find(|(_, n, _)| *n == name)
            .map(|(command, _, _)| command.clone())
    }

    // Every variant is listed, so that a new command has to choose its target.
    #[allow(clippy::too_many_lines)]
    fn target(&self) -> Target {
        match self {
            Self::PreviewClose => Target::AnyPreview,
            Self::PreviewScrollUp
            | Self::PreviewScrollDown
            | Self::PreviewPageUp
            | Self::PreviewPageDown
            | Self::PreviewTop
            | Self::PreviewBottom
            | Self::PreviewToggleMode
            | Self::PreviewEncodings
            | Self::PreviewEdit => Target::TextPreview,
            Self::LogShrink
            | Self::LogGrow
            | Self::LogScrollUp
            | Self::LogScrollDown
            | Self::LogPageUp
            | Self::LogPageDown
            | Self::LogTop
            | Self::LogBottom
            | Self::LogClear
            | Self::LogCopy => Target::Log,
            Self::Palette
            | Self::NewWindow
            | Self::NewWindowAsAdmin
            | Self::Quit
            | Self::QuitWithoutSaving
            | Self::EditConfig => Target::App,
            Self::SplitLeft | Self::SplitRight => Target::Split,
            Self::CursorUp
            | Self::CursorDown
            | Self::CursorPageUp
            | Self::CursorPageDown
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
            | Self::CursorNextMark
            | Self::CursorPrevMark
            | Self::ClearMarks => Target::Cursor,
            Self::Sort | Self::ToggleHidden | Self::Filter | Self::ClearFilter | Self::Reload => {
                Target::ListView
            }
            Self::Open
            | Self::OpenAlternative
            | Self::OpenAsText
            | Self::GoParent
            | Self::GoRoot
            | Self::GoHome
            | Self::GoToPath
            | Self::Browse
            | Self::Find
            | Self::Bookmarks
            | Self::History
            | Self::GoBack
            | Self::GoForward
            | Self::Drives
            | Self::GoPrevDrive
            | Self::GoNextDrive
            | Self::FocusLeft
            | Self::FocusRight
            | Self::FocusOther
            | Self::SyncToOther
            | Self::SyncFromOther
            | Self::SwapPanes
            | Self::OpenAsArchive => Target::Location,
            // `nop` only unbinds a key, which the configuration allows in
            // every context; in the palette it stays with the file list.
            Self::Copy
            | Self::Move
            | Self::Delete
            | Self::Trash
            | Self::Rename
            | Self::RenameReplace
            | Self::RenameTemplate
            | Self::NewDir
            | Self::NewFile
            | Self::NewText
            | Self::Edit
            | Self::Run
            | Self::RunCommandLine
            | Self::Compress
            | Self::CompressHere
            | Self::Extract
            | Self::ExtractHere
            | Self::CalculateSize
            | Self::CalculateSizeOnDisk
            | Self::ShellMenu
            | Self::OpenWithOs
            | Self::CopyPath
            | Self::CopyName
            | Self::CopyFiles
            | Self::PasteFiles
            | Self::MarkCompare
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
                    .find(|(command, _, _)| command == self)
                    .map_or("?", |(_, name, _)| name);
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
            (Command::PreviewClose, &[Context::Image, Context::Text][..]),
            (Command::PreviewScrollDown, &[Context::Text]),
            (
                Command::LogClear,
                &[Context::FileList, Context::Image, Context::Text],
            ),
            (
                Command::Quit,
                &[Context::FileList, Context::Image, Context::Text],
            ),
            (Command::SplitRight, &[Context::FileList, Context::Image]),
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
