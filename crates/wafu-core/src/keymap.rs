use std::collections::HashMap;

use crate::app::Os;
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
    ("F5", "reload"),
    ("F1", "palette"),
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
    /// The default bindings for the OS. The image preview reuses the file
    /// list's cursor and mark keys; the text and binary previews have fixed
    /// keys and no bindings.
    #[must_use]
    pub fn default_for(os: Os) -> Self {
        let jumps = Command::all().filter_map(|command| match command {
            Command::Jump(c) => Some((format!("Ctrl+{}", c.to_ascii_uppercase()), command)),
            _ => None,
        });
        let filelist: HashMap<Key, Command> = DEFAULT_FILELIST
            .iter()
            .map(|(key, name)| {
                let command = Command::from_name(name)
                    .unwrap_or_else(|| panic!("unknown command in the default key map: {name}"));
                ((*key).to_owned(), command)
            })
            .chain(jumps)
            .filter(|(_, command)| command.available_on(os))
            .map(|(key, command)| {
                let key: Key = key
                    .parse()
                    .unwrap_or_else(|_| panic!("invalid key in the default key map: {key}"));
                (key, command)
            })
            .collect();
        let image = filelist
            .iter()
            .filter(|(_, command)| command.available_in(Context::Image))
            .map(|(key, command)| (*key, command.clone()))
            .collect();
        Self {
            maps: HashMap::from([
                (Context::FileList, filelist),
                (Context::Image, image),
                (Context::Text, HashMap::new()),
                (Context::Binary, HashMap::new()),
            ]),
        }
    }

    #[must_use]
    pub fn lookup(&self, context: Context, key: &Key) -> Option<Command> {
        self.maps.get(&context)?.get(key).cloned()
    }

    /// The keys bound to a command in a context, in display order.
    #[must_use]
    pub fn keys_for(&self, context: Context, command: &Command) -> Vec<Key> {
        let mut keys: Vec<Key> = self
            .bindings(context)
            .filter(|(_, bound)| *bound == command)
            .map(|(key, _)| *key)
            .collect();
        keys.sort_by_key(ToString::to_string);
        keys
    }

    /// The bindings of a context, in no particular order.
    pub fn bindings(&self, context: Context) -> impl Iterator<Item = (&Key, &Command)> {
        self.maps
            .get(&context)
            .into_iter()
            .flat_map(|map| map.iter())
    }

    /// Binds a key, replacing what it was bound to, or unbinds it with `None`.
    pub fn bind(&mut self, context: Context, key: Key, command: Option<Command>) {
        let map = self.maps.entry(context).or_default();
        match command {
            Some(command) => {
                map.insert(key, command);
            }
            None => {
                map.remove(&key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    const UNBOUND: [Command; 8] = [
        Command::SwapPanes,
        Command::ClearMarks,
        Command::NewFile,
        Command::CopyPath,
        Command::CopyName,
        Command::ClearLog,
        Command::CopyLog,
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
        let keymap = KeyMap::default_for(Os::Windows);
        let bound: HashSet<Command> = keymap
            .bindings(Context::FileList)
            .map(|(_, command)| command.clone())
            .collect();

        for command in Command::all() {
            let expected = !UNBOUND.contains(&command);
            assert_eq!(bound.contains(&command), expected, "{command}");
        }
    }

    #[test]
    fn default_binding_count_matches_the_table() {
        let keymap = KeyMap::default_for(Os::Windows);

        assert_eq!(
            keymap.bindings(Context::FileList).count(),
            DEFAULT_FILELIST.len() + 36
        );
    }

    #[test]
    fn windows_only_keys_are_left_unbound_on_other_systems() {
        for os in [Os::MacOs, Os::Linux] {
            let keymap = KeyMap::default_for(os);

            assert_eq!(
                keymap.bindings(Context::FileList).count(),
                DEFAULT_FILELIST.len() + 36 - 4
            );
            for key in ["L", "Ctrl+Left", "Ctrl+Right", "\\"] {
                assert_eq!(
                    keymap.lookup(Context::FileList, &key.parse().unwrap()),
                    None,
                    "{key} on {os:?}"
                );
            }
        }
    }

    #[test]
    fn image_context_has_the_cursor_and_mark_keys_only() {
        let keymap = KeyMap::default_for(Os::Windows);

        // The sixteen commands minus clear_marks, which has no key.
        assert_eq!(keymap.bindings(Context::Image).count(), 15);
        assert_eq!(
            keymap.lookup(Context::Image, &"Up".parse().unwrap()),
            Some(Command::CursorUp)
        );
        assert_eq!(keymap.lookup(Context::Image, &"C".parse().unwrap()), None);
        assert_eq!(keymap.bindings(Context::Text).count(), 0);
    }

    #[test]
    fn bind_replaces_and_unbinds() {
        let mut keymap = KeyMap::default_for(Os::Windows);
        let key: Key = "Ctrl+C".parse().unwrap();

        keymap.bind(Context::FileList, key, Some(Command::CopyPath));
        assert_eq!(
            keymap.lookup(Context::FileList, &key),
            Some(Command::CopyPath)
        );
        assert_eq!(
            keymap.keys_for(Context::FileList, &Command::Jump('c')),
            Vec::new()
        );

        keymap.bind(Context::FileList, key, None);
        assert_eq!(keymap.lookup(Context::FileList, &key), None);
    }

    #[test]
    fn lookup_finds_bound_keys() {
        let keymap = KeyMap::default_for(Os::Windows);

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
        let keymap = KeyMap::default_for(Os::Windows);

        let keys: Vec<String> = keymap
            .keys_for(Context::FileList, &Command::MarkRangeBelow)
            .iter()
            .map(ToString::to_string)
            .collect();
        assert_eq!(keys, ["Ctrl+Shift+Space"]);
        assert_eq!(
            keymap.keys_for(Context::FileList, &Command::SwapPanes),
            Vec::new()
        );
    }

    #[test]
    fn lookup_misses_unbound_keys_and_other_contexts() {
        let keymap = KeyMap::default_for(Os::Windows);

        assert_eq!(
            keymap.lookup(Context::FileList, &"F2".parse().unwrap()),
            None
        );
        assert_eq!(keymap.lookup(Context::Text, &"Up".parse().unwrap()), None);
    }
}
