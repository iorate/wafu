use std::collections::HashMap;

use crate::command::{Command, Context};
use crate::key::Key;

/// Key bindings per context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMap {
    maps: HashMap<Context, HashMap<Key, Command>>,
}

// The jump commands are generated in `Default`.
const DEFAULT_FILELIST: &[(&str, &str)] = &[
    ("Up", "cursor_up"),
    ("Down", "cursor_down"),
    ("PageUp", "page_up"),
    ("PageDown", "page_down"),
    ("Home", "cursor_top"),
    ("End", "cursor_bottom"),
    ("Left", "focus_left"),
    ("Right", "focus_right"),
    ("Tab", "switch_pane"),
    ("Enter", "open"),
    ("Ctrl+Enter", "open_with_os"),
    ("BS", "go_parent"),
    ("/", "go_root"),
    ("~", "go_home"),
    ("Shift+J", "go_to_path"),
    ("J", "bookmarks"),
    ("H", "history"),
    ("Ctrl+Shift+Left", "history_back"),
    ("Ctrl+Shift+Right", "history_forward"),
    ("L", "drives"),
    ("Ctrl+Left", "drive_prev"),
    ("Ctrl+Right", "drive_next"),
    ("O", "sync_to_other"),
    ("Shift+O", "sync_other_to_this"),
    ("Space", "mark_down"),
    ("Shift+Space", "mark_up"),
    ("Ctrl+Space", "mark_range_above"),
    ("Ctrl+Shift+Space", "mark_range_below"),
    ("A", "invert_marks"),
    ("Shift+A", "invert_marks_all"),
    ("@", "mark_glob"),
    ("Ctrl+:", "next_mark"),
    ("Ctrl+*", "prev_mark"),
    ("C", "copy"),
    ("M", "move"),
    ("D", "delete"),
    ("Shift+D", "trash"),
    ("R", "rename"),
    ("K", "mkdir"),
    ("E", "edit"),
    ("Shift+E", "new_text"),
    ("P", "compress"),
    ("U", "extract"),
    ("Shift+U", "enter_as_archive"),
    ("\\", "shell_menu"),
    ("S", "sort"),
    ("F", "search"),
    (":", "filter"),
    ("*", "clear_filter"),
    (".", "toggle_hidden"),
    ("Y", "reload"),
    ("B", "palette"),
    ("Z", "edit_config"),
    ("Alt+Z", "new_window"),
    ("Q", "quit"),
    ("Shift+Q", "quit_without_saving"),
    ("Alt+Left", "pane_shrink_left"),
    ("Alt+Right", "pane_grow_left"),
    ("Alt+Up", "log_grow"),
    ("Alt+Down", "log_shrink"),
    ("Shift+Up", "log_scroll_up"),
    ("Shift+Down", "log_scroll_down"),
    ("Shift+PageUp", "log_page_up"),
    ("Shift+PageDown", "log_page_down"),
];

impl KeyMap {
    #[must_use]
    pub fn lookup(&self, context: Context, key: &Key) -> Option<Command> {
        self.maps.get(&context)?.get(key).copied()
    }

    /// The keys bound to a command in a context, in display order.
    #[must_use]
    pub fn keys_for(&self, context: Context, command: Command) -> Vec<Key> {
        let mut keys: Vec<Key> = self
            .bindings(context)
            .filter(|(_, bound)| *bound == command)
            .map(|(key, _)| *key)
            .collect();
        keys.sort_by_key(ToString::to_string);
        keys
    }

    /// The bindings of a context, in no particular order.
    pub fn bindings(&self, context: Context) -> impl Iterator<Item = (&Key, Command)> {
        self.maps
            .get(&context)
            .into_iter()
            .flat_map(|map| map.iter().map(|(key, command)| (key, *command)))
    }
}

impl Default for KeyMap {
    fn default() -> Self {
        let jumps = "abcdefghijklmnopqrstuvwxyz0123456789"
            .chars()
            .map(|c| (format!("Ctrl+{}", c.to_ascii_uppercase()), Command::Jump(c)));
        let filelist = DEFAULT_FILELIST
            .iter()
            .map(|(key, name)| {
                let command = Command::from_name(name)
                    .unwrap_or_else(|| panic!("unknown command in the default key map: {name}"));
                ((*key).to_owned(), command)
            })
            .chain(jumps)
            .map(|(key, command)| {
                let key: Key = key
                    .parse()
                    .unwrap_or_else(|_| panic!("invalid key in the default key map: {key}"));
                (key, command)
            })
            .collect();
        Self {
            maps: HashMap::from([(Context::FileList, filelist)]),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const UNBOUND: [Command; 7] = [
        Command::SwapPanes,
        Command::ClearMarks,
        Command::NewFile,
        Command::CopyPath,
        Command::CopyName,
        Command::ClearLog,
        Command::Nop,
    ];

    #[test]
    fn default_has_no_duplicate_keys() {
        let mut seen = HashSet::new();
        for (key, _) in DEFAULT_FILELIST {
            assert!(
                seen.insert(key.parse::<Key>().unwrap()),
                "{key} is bound twice"
            );
        }
    }

    #[test]
    fn default_binds_every_command_except_the_unbound_ones() {
        let keymap = KeyMap::default();
        let bound: HashSet<Command> = keymap
            .bindings(Context::FileList)
            .map(|(_, command)| command)
            .collect();

        for command in Command::all() {
            let expected = !UNBOUND.contains(&command);
            assert_eq!(bound.contains(&command), expected, "{}", command.name());
        }
    }

    #[test]
    fn default_binding_count_matches_the_table() {
        let keymap = KeyMap::default();

        assert_eq!(
            keymap.bindings(Context::FileList).count(),
            DEFAULT_FILELIST.len() + 36
        );
    }

    #[test]
    fn lookup_finds_bound_keys() {
        let keymap = KeyMap::default();

        assert_eq!(
            keymap.lookup(Context::FileList, &"Ctrl+Shift+Left".parse().unwrap()),
            Some(Command::HistoryBack)
        );
        assert_eq!(
            keymap.lookup(Context::FileList, &"Ctrl+Q".parse().unwrap()),
            Some(Command::Jump('q'))
        );
        assert_eq!(
            keymap.lookup(Context::FileList, &"Ctrl+7".parse().unwrap()),
            Some(Command::Jump('7'))
        );
    }

    #[test]
    fn keys_for_lists_bound_keys_in_display_order() {
        let keymap = KeyMap::default();

        let keys: Vec<String> = keymap
            .keys_for(Context::FileList, Command::MarkRangeBelow)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(keys, ["Ctrl+Shift+Space"]);
        assert_eq!(
            keymap.keys_for(Context::FileList, Command::SwapPanes),
            Vec::new()
        );
    }

    #[test]
    fn lookup_misses_unbound_keys_and_other_contexts() {
        let keymap = KeyMap::default();

        assert_eq!(
            keymap.lookup(Context::FileList, &"F1".parse().unwrap()),
            None
        );
        assert_eq!(keymap.lookup(Context::Image, &"Up".parse().unwrap()), None);
    }
}
