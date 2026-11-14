use std::collections::HashMap;

use crate::app::Os;
use crate::command::{Command, Context};
use crate::key::Key;

// The jump commands are generated in `default_for`, for the file list and the image preview.
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
    ("Shift+Enter", "open_alternative"),
    ("Ctrl+Enter", "open_with_os"),
    ("V", "open_as_text"),
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
    ("X", "run_file"),
    ("Shift+X", "run_command"),
    ("P", "compress"),
    ("U", "extract"),
    ("Shift+U", "enter_as_archive"),
    ("I", "calculate_size_on_disk"),
    ("Shift+I", "calculate_size"),
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
    ("Alt+Shift+Z", "new_window_as_admin"),
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

const DEFAULT_IMAGE: &[(&str, &str)] = &[
    ("Up", "cursor_up"),
    ("Down", "cursor_down"),
    ("PageUp", "page_up"),
    ("PageDown", "page_down"),
    ("Home", "cursor_top"),
    ("End", "cursor_bottom"),
    ("Space", "mark_down"),
    ("Shift+Space", "mark_up"),
    ("Ctrl+Space", "mark_range_above"),
    ("Ctrl+Shift+Space", "mark_range_below"),
    ("A", "invert_marks"),
    ("Shift+A", "invert_marks_all"),
    ("@", "mark_glob"),
    ("Ctrl+:", "next_mark"),
    ("Ctrl+*", "prev_mark"),
    ("F", "search"),
    ("Enter", "close_preview"),
    ("Esc", "close_preview"),
    ("F1", "palette"),
    ("Alt+Z", "new_window"),
    ("Alt+Shift+Z", "new_window_as_admin"),
    ("Alt+Left", "pane_shrink_left"),
    ("Alt+Right", "pane_grow_left"),
    ("Alt+Up", "log_grow"),
    ("Alt+Down", "log_shrink"),
    ("Shift+Up", "log_scroll_up"),
    ("Shift+Down", "log_scroll_down"),
    ("Shift+PageUp", "log_page_up"),
    ("Shift+PageDown", "log_page_down"),
];

const DEFAULT_TEXT: &[(&str, &str)] = &[
    ("Up", "preview_scroll_up"),
    ("Down", "preview_scroll_down"),
    ("PageUp", "preview_page_up"),
    ("PageDown", "preview_page_down"),
    ("Home", "preview_top"),
    ("End", "preview_bottom"),
    ("Tab", "preview_toggle_mode"),
    ("C", "preview_encoding"),
    ("E", "preview_edit"),
    ("Enter", "close_preview"),
    ("Esc", "close_preview"),
    ("F1", "palette"),
    ("Alt+Z", "new_window"),
    ("Alt+Shift+Z", "new_window_as_admin"),
    ("Alt+Up", "log_grow"),
    ("Alt+Down", "log_shrink"),
    ("Shift+Up", "log_scroll_up"),
    ("Shift+Down", "log_scroll_down"),
    ("Shift+PageUp", "log_page_up"),
    ("Shift+PageDown", "log_page_down"),
];

/// Key bindings per context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyMap {
    maps: HashMap<Context, HashMap<Key, Command>>,
}

impl KeyMap {
    /// The default bindings for the OS, one table per context.
    #[must_use]
    pub fn default_for(os: Os) -> Self {
        let jumps = || {
            Command::all().filter_map(|command| match command {
                Command::Jump(c) => Some((format!("Ctrl+{}", c.to_ascii_uppercase()), command)),
                _ => None,
            })
        };
        Self {
            maps: HashMap::from([
                (
                    Context::FileList,
                    bindings(table(DEFAULT_FILELIST).chain(jumps()), os),
                ),
                (
                    Context::Image,
                    bindings(table(DEFAULT_IMAGE).chain(jumps()), os),
                ),
                (Context::Text, bindings(table(DEFAULT_TEXT), os)),
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

fn table(
    entries: &'static [(&'static str, &'static str)],
) -> impl Iterator<Item = (String, Command)> {
    entries
        .iter()
        .map(|(key, name)| ((*key).to_owned(), named(name)))
}

fn named(name: &str) -> Command {
    Command::from_name(name)
        .unwrap_or_else(|| panic!("unknown command in the default key map: {name}"))
}

fn bindings(entries: impl Iterator<Item = (String, Command)>, os: Os) -> HashMap<Key, Command> {
    entries
        .filter(|(_, command)| command.available_on(os))
        .map(|(key, command)| {
            let key: Key = key
                .parse()
                .unwrap_or_else(|_| panic!("invalid key in the default key map: {key}"));
            (key, command)
        })
        .collect()
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
    fn default_binds_every_command_except_the_unbound_ones() {
        let keymap = KeyMap::default_for(Os::Windows);
        let bound: HashSet<Command> = keymap
            .bindings(Context::FileList)
            .map(|(_, command)| command.clone())
            .collect();

        for command in Command::all() {
            let expected = !UNBOUND.contains(&command) && command.available_in(Context::FileList);
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
        assert_eq!(
            keymap.bindings(Context::Image).count(),
            DEFAULT_IMAGE.len() + 36
        );
        assert_eq!(keymap.bindings(Context::Text).count(), DEFAULT_TEXT.len());
    }

    #[test]
    fn windows_only_keys_are_left_unbound_on_other_systems() {
        for os in [Os::MacOs, Os::Linux] {
            let keymap = KeyMap::default_for(os);

            assert_eq!(
                keymap.bindings(Context::FileList).count(),
                DEFAULT_FILELIST.len() + 36 - 5
            );
            assert_eq!(
                keymap.bindings(Context::Image).count(),
                DEFAULT_IMAGE.len() + 36 - 1
            );
            assert_eq!(
                keymap.bindings(Context::Text).count(),
                DEFAULT_TEXT.len() - 1
            );
            for key in ["L", "Ctrl+Left", "Ctrl+Right", "\\", "Alt+Shift+Z"] {
                assert_eq!(
                    keymap.lookup(Context::FileList, &key.parse().unwrap()),
                    None,
                    "{key} on {os:?}"
                );
            }
        }
    }

    #[test]
    fn keys_for_lists_the_keys_in_display_order() {
        let mut keymap = KeyMap::default_for(Os::Windows);
        for key in ["Ctrl+R", "Shift+F5", "Alt+R"] {
            keymap.bind(
                Context::FileList,
                key.parse().unwrap(),
                Some(Command::Reload),
            );
        }

        assert_eq!(
            key_names(&keymap, Context::FileList, &Command::Reload),
            ["Alt+R", "Ctrl+R", "F5", "Shift+F5"]
        );
    }

    #[test]
    fn keys_for_leaves_out_keys_bound_in_another_context() {
        let mut keymap = KeyMap::default_for(Os::Windows);
        keymap.bind(
            Context::Text,
            "Ctrl+Q".parse().unwrap(),
            Some(Command::LogGrow),
        );

        assert_eq!(
            key_names(&keymap, Context::FileList, &Command::LogGrow),
            ["Alt+Up"]
        );
    }

    #[test]
    fn every_default_binding_is_available_in_its_context() {
        let keymap = KeyMap::default_for(Os::Windows);

        for context in [Context::FileList, Context::Image, Context::Text] {
            for (key, command) in keymap.bindings(context) {
                assert!(
                    command.available_in(context),
                    "{command} on {key} in {context:?}"
                );
            }
        }
    }

    fn key_names(keymap: &KeyMap, context: Context, command: &Command) -> Vec<String> {
        keymap
            .keys_for(context, command)
            .iter()
            .map(ToString::to_string)
            .collect()
    }
}
