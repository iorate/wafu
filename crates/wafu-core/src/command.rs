use std::fmt;

/// A named operation. Keys, the mouse, and the palette all invoke these.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Command {
    Open,
    OpenWithOs,
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
    Copy,
    Move,
    Delete,
    Trash,
    Rename,
    Mkdir,
    NewFile,
    NewText,
    Edit,
    CopyPath,
    CopyName,
    ShellMenu,
    EnterAsArchive,
    Extract,
    Compress,
    Palette,
    EditConfig,
    NewWindow,
    Quit,
    QuitWithoutSaving,
    Nop,
    /// A command defined in the configuration, named `$name` in key bindings.
    External(String),
}

/// Where a key press is interpreted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Context {
    FileList,
    Image,
    Text,
    Binary,
}

const JUMP_NAMES: [&str; 36] = [
    "jump_a", "jump_b", "jump_c", "jump_d", "jump_e", "jump_f", "jump_g", "jump_h", "jump_i",
    "jump_j", "jump_k", "jump_l", "jump_m", "jump_n", "jump_o", "jump_p", "jump_q", "jump_r",
    "jump_s", "jump_t", "jump_u", "jump_v", "jump_w", "jump_x", "jump_y", "jump_z", "jump_0",
    "jump_1", "jump_2", "jump_3", "jump_4", "jump_5", "jump_6", "jump_7", "jump_8", "jump_9",
];

const NAMES: &[(Command, &str)] = &[
    (Command::Open, "open"),
    (Command::OpenWithOs, "open_with_os"),
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
    (Command::Copy, "copy"),
    (Command::Move, "move"),
    (Command::Delete, "delete"),
    (Command::Trash, "trash"),
    (Command::Rename, "rename"),
    (Command::Mkdir, "mkdir"),
    (Command::NewFile, "new_file"),
    (Command::NewText, "new_text"),
    (Command::Edit, "edit"),
    (Command::CopyPath, "copy_path"),
    (Command::CopyName, "copy_name"),
    (Command::ShellMenu, "shell_menu"),
    (Command::EnterAsArchive, "enter_as_archive"),
    (Command::Extract, "extract"),
    (Command::Compress, "compress"),
    (Command::Palette, "palette"),
    (Command::EditConfig, "edit_config"),
    (Command::NewWindow, "new_window"),
    (Command::Quit, "quit"),
    (Command::QuitWithoutSaving, "quit_without_saving"),
    (Command::Nop, "nop"),
];

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
            .chain(JUMP_NAMES.iter().map(|name| Self::Jump(jump_char(name))))
            .chain(after.iter().map(|(command, _)| command.clone()))
    }

    /// Whether the command can run in the context. Commands that do nothing on
    /// some OS still count as available, so that a shared configuration loads
    /// without errors everywhere.
    #[must_use]
    pub fn available_in(&self, context: Context) -> bool {
        match context {
            Context::FileList => true,
            Context::Image => matches!(
                self,
                Self::CursorUp
                    | Self::CursorDown
                    | Self::PageUp
                    | Self::PageDown
                    | Self::CursorTop
                    | Self::CursorBottom
                    | Self::MarkDown
                    | Self::MarkUp
                    | Self::MarkRangeAbove
                    | Self::MarkRangeBelow
                    | Self::InvertMarks
                    | Self::InvertMarksAll
                    | Self::MarkGlob
                    | Self::NextMark
                    | Self::PrevMark
                    | Self::ClearMarks
            ),
            Context::Text | Context::Binary => false,
        }
    }

    /// The command for a name as written in the configuration. `$name` is an
    /// external command; whether it is defined is checked elsewhere.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        if let Some(external) = name.strip_prefix('$') {
            return (!external.is_empty()).then(|| Self::External(external.to_owned()));
        }
        if let Some(name) = JUMP_NAMES.iter().find(|&&n| n == name) {
            return Some(Self::Jump(jump_char(name)));
        }
        NAMES
            .iter()
            .find(|(_, n)| *n == name)
            .map(|(command, _)| command.clone())
    }
}

impl fmt::Display for Command {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Jump(c) => {
                let name = JUMP_NAMES
                    .iter()
                    .find(|name| jump_char(name) == *c)
                    .map_or("jump_?", |name| name);
                f.write_str(name)
            }
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

fn jump_char(name: &str) -> char {
    name.chars().next_back().unwrap_or('?')
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

        assert_eq!(commands.len(), NAMES.len() + JUMP_NAMES.len());
        assert_eq!(unique.len(), commands.len());
    }

    #[test]
    fn jump_names_follow_the_character() {
        assert_eq!(Command::Jump('a').to_string(), "jump_a");
        assert_eq!(Command::Jump('9').to_string(), "jump_9");
        assert_eq!(Command::from_name("jump_z"), Some(Command::Jump('z')));
        assert_eq!(Command::from_name("jump_A"), None);
    }

    #[test]
    fn availability_follows_the_context() {
        assert!(Command::Copy.available_in(Context::FileList));
        assert!(Command::Drives.available_in(Context::FileList));
        assert!(Command::CursorDown.available_in(Context::Image));
        assert!(Command::MarkDown.available_in(Context::Image));
        assert!(!Command::Copy.available_in(Context::Image));
        assert!(!Command::CursorDown.available_in(Context::Text));
        assert_eq!(
            Command::all()
                .filter(|c| c.available_in(Context::Image))
                .count(),
            16
        );
    }

    #[test]
    fn external_names_start_with_a_dollar() {
        let command = Command::External("open_terminal".to_owned());

        assert_eq!(Command::from_name("$open_terminal"), Some(command.clone()));
        assert_eq!(command.to_string(), "$open_terminal");
        assert!(command.available_in(Context::FileList));
        assert!(!command.available_in(Context::Image));
        assert!(!Command::all().any(|c| matches!(c, Command::External(_))));
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert_eq!(Command::from_name(""), None);
        assert_eq!(Command::from_name("Open"), None);
        assert_eq!(Command::from_name("$"), None);
    }
}
