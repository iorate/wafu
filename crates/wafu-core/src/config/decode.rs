use std::ops::Range;
use std::path::PathBuf;

use toml::{Table, Value};

use super::{
    Color, Colors, Config, Diagnostic, DiagnosticKind, FONT_SIZE_MAX, FONT_SIZE_MIN, Font,
    SizeFormat, SyntaxError, Theme,
};
use crate::app::Os;
use crate::command::{Command, Context};
use crate::external::CommandLine;
use crate::key::Key;
use crate::listing::SortKey;

const CONTEXTS: [(&str, Context); 4] = [
    ("filelist", Context::FileList),
    ("image", Context::Image),
    ("text", Context::Text),
    ("binary", Context::Binary),
];

pub(super) fn parse(text: &str, os: Os) -> Result<(Config, Vec<Diagnostic>), SyntaxError> {
    let mut table: Table = toml::from_str(text).map_err(|error| SyntaxError {
        line: line_of(text, error.span()),
        message: error.message().to_owned(),
    })?;
    let mut diagnostics = Vec::new();
    merge_os_section(&mut table, os, &mut diagnostics);
    let mut config = Config::default_for(os);
    let mut root = Reader::new(table, String::new(), &mut diagnostics);
    read_general(&mut root, &mut config);
    read_appearance(&mut root, &mut config);
    read_colors(&mut root, &mut config);
    read_initial(&mut root, &mut config);
    read_bookmarks(&mut root, &mut config);
    read_commands(&mut root, &mut config);
    read_keys(&mut root, &mut config);
    root.finish();
    Ok((config, diagnostics))
}

// Walks a table, taking each item out as it is read so that whatever is left
// at the end is unknown.
struct Reader<'a> {
    table: Table,
    path: String,
    diagnostics: &'a mut Vec<Diagnostic>,
}

impl<'a> Reader<'a> {
    fn new(table: Table, path: String, diagnostics: &'a mut Vec<Diagnostic>) -> Self {
        Self {
            table,
            path,
            diagnostics,
        }
    }

    fn path_of(&self, key: &str) -> String {
        if self.path.is_empty() {
            key.to_owned()
        } else {
            format!("{}.{key}", self.path)
        }
    }

    fn report(&mut self, path: String, kind: DiagnosticKind) {
        self.diagnostics.push(Diagnostic { path, kind });
    }

    fn invalid(&mut self, key: &str, expected: &str) {
        let path = self.path_of(key);
        self.report(
            path,
            DiagnosticKind::InvalidValue {
                expected: expected.to_owned(),
            },
        );
    }

    fn table(&mut self, key: &str) -> Option<Reader<'_>> {
        if let Value::Table(table) = self.table.remove(key)? {
            let path = self.path_of(key);
            Some(Reader::new(table, path, self.diagnostics))
        } else {
            self.invalid(key, "a table");
            None
        }
    }

    fn string(&mut self, key: &str) -> Option<String> {
        if let Value::String(text) = self.table.remove(key)? {
            Some(text)
        } else {
            self.invalid(key, "a string");
            None
        }
    }

    fn bool(&mut self, key: &str) -> Option<bool> {
        if let Value::Boolean(value) = self.table.remove(key)? {
            Some(value)
        } else {
            self.invalid(key, "true or false");
            None
        }
    }

    // Reads a string and maps it; anything the mapping rejects is invalid.
    fn choice<T>(
        &mut self,
        key: &str,
        expected: &str,
        parse: impl FnOnce(&str) -> Option<T>,
    ) -> Option<T> {
        let value = match self.table.remove(key)? {
            Value::String(text) => parse(&text),
            _ => None,
        };
        if value.is_none() {
            self.invalid(key, expected);
        }
        value
    }

    fn size(&mut self, key: &str) -> Option<u16> {
        let expected = format!("an integer from {FONT_SIZE_MIN} to {FONT_SIZE_MAX}");
        match self.table.remove(key)? {
            Value::Integer(value)
                if (i64::from(FONT_SIZE_MIN)..=i64::from(FONT_SIZE_MAX)).contains(&value) =>
            {
                u16::try_from(value).ok()
            }
            _ => {
                self.invalid(key, &expected);
                None
            }
        }
    }

    fn color(&mut self, key: &str) -> Option<Color> {
        self.choice(key, "a color like #rrggbb", Color::parse)
    }

    /// Takes every remaining item, for tables whose keys are user-defined.
    fn entries(&mut self) -> Vec<(String, Value)> {
        std::mem::take(&mut self.table).into_iter().collect()
    }

    fn finish(mut self) {
        let unknown: Vec<String> = self.table.keys().map(|key| self.path_of(key)).collect();
        for path in unknown {
            self.report(path, DiagnosticKind::UnknownItem);
        }
    }
}

fn read_general(root: &mut Reader<'_>, config: &mut Config) {
    let Some(mut general) = root.table("general") else {
        return;
    };
    if let Some(value) = general.bool("mkdir_then_enter") {
        config.general.mkdir_then_enter = value;
    }
    if let Some(value) = general.bool("new_text_creates_file") {
        config.general.new_text_creates_file = value;
    }
    if let Some(value) = general.bool("require_marks") {
        config.general.require_marks = value;
    }
    if let Some(value) = general.bool("hide_log_in_preview") {
        config.general.hide_log_in_preview = value;
    }
    if let Some(text) = general.string("editor") {
        match CommandLine::parse(&text) {
            Ok(editor) => config.general.editor = editor,
            Err(error) => {
                let path = general.path_of("editor");
                general.report(path, DiagnosticKind::InvalidCommandLine(error));
            }
        }
    }
    general.finish();
}

fn read_appearance(root: &mut Reader<'_>, config: &mut Config) {
    let Some(mut appearance) = root.table("appearance") else {
        return;
    };
    if let Some(theme) = appearance.choice("theme", "auto, light, or dark", Theme::from_name) {
        config.appearance.theme = theme;
    }
    if let Some(format) = appearance.choice(
        "size_format",
        "plain, comma, or human",
        SizeFormat::from_name,
    ) {
        config.appearance.size_format = format;
    }
    if let Some(mut font) = appearance.table("font") {
        let fonts = &mut config.appearance.font;
        for (key, area) in [
            ("filelist", &mut fonts.filelist),
            ("log", &mut fonts.log),
            ("preview", &mut fonts.preview),
        ] {
            if let Some(mut reader) = font.table(key) {
                read_font(&mut reader, area);
                reader.finish();
            }
        }
        read_font(&mut font, &mut fonts.default);
        font.finish();
    }
    appearance.finish();
}

fn read_font(reader: &mut Reader<'_>, font: &mut Font) {
    if let Some(family) = reader.string("family") {
        font.family = (!family.is_empty()).then_some(family);
    }
    if let Some(size) = reader.size("size") {
        font.size = Some(size);
    }
}

fn read_colors(root: &mut Reader<'_>, config: &mut Config) {
    let Some(mut colors) = root.table("colors") else {
        return;
    };
    for (key, set) in [
        ("light", &mut config.colors.light),
        ("dark", &mut config.colors.dark),
    ] {
        let Some(mut reader) = colors.table(key) else {
            continue;
        };
        let Colors {
            fg,
            bg,
            mark,
            cursor,
            dir,
            link,
            exec,
            broken,
            fifo,
            socket,
        } = set;
        for (item, color) in [
            ("fg", fg),
            ("bg", bg),
            ("mark", mark),
            ("cursor", cursor),
            ("dir", dir),
            ("link", link),
            ("exec", exec),
            ("broken", broken),
            ("fifo", fifo),
            ("socket", socket),
        ] {
            if let Some(value) = reader.color(item) {
                *color = value;
            }
        }
        reader.finish();
    }
    colors.finish();
}

fn read_initial(root: &mut Reader<'_>, config: &mut Config) {
    let Some(mut initial) = root.table("initial") else {
        return;
    };
    if let Some(key) = initial.choice(
        "sort",
        "none, name, extension, size, time, or natural",
        SortKey::from_name,
    ) {
        config.initial.sort.key = key;
    }
    let sort = &mut config.initial.sort;
    for (item, flag) in [
        ("descending", &mut sort.descending),
        ("dirs_first", &mut sort.dirs_first),
        ("ignore_case", &mut sort.ignore_case),
        ("show_hidden", &mut config.initial.show_hidden),
    ] {
        if let Some(value) = initial.bool(item) {
            *flag = value;
        }
    }
    initial.finish();
}

fn read_bookmarks(root: &mut Reader<'_>, config: &mut Config) {
    let Some(value) = root.table.remove("bookmarks") else {
        return;
    };
    let Value::Array(items) = value else {
        root.invalid("bookmarks", "an array of tables");
        return;
    };
    for (index, item) in items.into_iter().enumerate() {
        let key = format!("bookmarks[{index}]");
        let Value::Table(table) = item else {
            root.invalid(&key, "a table");
            continue;
        };
        let path = root.path_of(&key);
        let mut reader = Reader::new(table, path, root.diagnostics);
        if let Some(path) = reader.string("path") {
            config.bookmarks.push(PathBuf::from(path));
        }
        reader.finish();
    }
}

fn read_commands(root: &mut Reader<'_>, config: &mut Config) {
    let Some(mut commands) = root.table("commands") else {
        return;
    };
    for (name, value) in commands.entries() {
        let Value::Table(table) = value else {
            commands.invalid(&name, "a table");
            continue;
        };
        let path = commands.path_of(&name);
        let mut reader = Reader::new(table, path, commands.diagnostics);
        if let Some(run) = reader.string("run") {
            match CommandLine::parse(&run) {
                Ok(line) => {
                    config.commands.insert(name, line);
                }
                Err(error) => {
                    let path = reader.path_of("run");
                    reader.report(path, DiagnosticKind::InvalidCommandLine(error));
                }
            }
        }
        reader.finish();
    }
    commands.finish();
}

fn read_keys(root: &mut Reader<'_>, config: &mut Config) {
    let Some(mut keys) = root.table("keys") else {
        return;
    };
    for (name, context) in CONTEXTS {
        let Some(mut reader) = keys.table(name) else {
            continue;
        };
        for (text, value) in reader.entries() {
            let Ok(key) = text.parse::<Key>() else {
                let path = reader.path.clone();
                reader.report(path, DiagnosticKind::InvalidKey { key: text });
                continue;
            };
            let Value::String(name) = value else {
                reader.invalid(&text, "a command name");
                continue;
            };
            let path = reader.path_of(&text);
            let command = match Command::from_name(&name) {
                Some(Command::External(external)) if !config.commands.contains_key(&external) => {
                    None
                }
                other => other,
            };
            let Some(command) = command else {
                reader.report(path, DiagnosticKind::UnknownCommand { command: name });
                continue;
            };
            if command == Command::Nop {
                config.keys.bind(context, key, None);
            } else if command.available_in(context) {
                config.keys.bind(context, key, Some(command));
            } else {
                reader.report(path, DiagnosticKind::Unavailable { command: name });
            }
        }
        reader.finish();
    }
    keys.finish();
}

// The section of the running OS overrides the common items; the sections of
// the other systems are dropped unread, since their values are not for here.
fn merge_os_section(table: &mut Table, os: Os, diagnostics: &mut Vec<Diagnostic>) {
    let Some(value) = table.remove("os") else {
        return;
    };
    let Value::Table(mut sections) = value else {
        diagnostics.push(Diagnostic {
            path: "os".to_owned(),
            kind: DiagnosticKind::InvalidValue {
                expected: "a table".to_owned(),
            },
        });
        return;
    };
    for name in ["windows", "macos", "linux"] {
        let Some(section) = sections.remove(name) else {
            continue;
        };
        if name != os_name(os) {
            continue;
        }
        match section {
            Value::Table(section) => merge_into(table, section),
            _ => diagnostics.push(Diagnostic {
                path: format!("os.{name}"),
                kind: DiagnosticKind::InvalidValue {
                    expected: "a table".to_owned(),
                },
            }),
        }
    }
    for key in sections.keys() {
        diagnostics.push(Diagnostic {
            path: format!("os.{key}"),
            kind: DiagnosticKind::UnknownItem,
        });
    }
}

fn merge_into(base: &mut Table, overlay: Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(below)), Value::Table(above)) => merge_into(below, above),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn os_name(os: Os) -> &'static str {
    match os {
        Os::Windows => "windows",
        Os::MacOs => "macos",
        Os::Linux => "linux",
    }
}

fn line_of(text: &str, span: Option<Range<usize>>) -> usize {
    let start = span.map_or(0, |span| span.start).min(text.len());
    text[..start].matches('\n').count() + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::CommandLineError;
    use crate::listing::SortSpec;

    const EXAMPLE: &str = r##"
[general]
mkdir_then_enter = true
new_text_creates_file = false
require_marks = false
editor = "notepad {paths}"
hide_log_in_preview = false

[appearance]
theme = "auto"                 # auto / light / dark
size_format = "human"          # plain / comma / human

[appearance.font]              # the default
family = "..."
size = 12

[appearance.font.filelist]     # per area (log and preview too)
family = "..."

[colors.light]
fg = "#000000"
bg = "#ffffff"
mark = "#ff0000"
cursor = "#000000"
dir = "#0000ff"
link = "#00ff00"

[colors.dark]
fg = "#ffffff"
bg = "#000000"
mark = "#ff0000"
cursor = "#ffffff"
dir = "#0000ff"
link = "#00ff00"

[initial]                      # when there is no state file
sort = "name"
descending = false
dirs_first = true
ignore_case = true
show_hidden = false

[[bookmarks]]
path = "~/Downloads"

[commands.open_terminal]
run = "wt -d {dir}"

[keys.filelist]
"F2" = "$open_terminal"
"Q" = "nop"

[keys.image]
[keys.text]
[keys.binary]

[os.macos.general]
editor = "open -t {paths}"

[os.macos.keys.filelist]
"Ctrl+Space" = "nop"
"##;

    #[test]
    fn the_example_from_the_documentation_reads_without_warnings() {
        let (config, diagnostics) = parse(EXAMPLE, Os::Linux).unwrap();

        assert_eq!(diagnostics, Vec::new());
        assert_eq!(
            config.general.editor,
            CommandLine::parse("notepad {paths}").unwrap()
        );
        assert_eq!(
            config.appearance.font.default.family,
            Some("...".to_owned())
        );
        assert_eq!(config.appearance.font.filelist.size, None);
        assert_eq!(config.colors.light.dir, Color::new(0x00, 0x00, 0xff));
        assert_eq!(config.bookmarks, [PathBuf::from("~/Downloads")]);
        assert_eq!(
            config.commands["open_terminal"],
            CommandLine::parse("wt -d {dir}").unwrap()
        );
        assert_eq!(
            config
                .keys
                .lookup(Context::FileList, &"F2".parse().unwrap()),
            Some(Command::External("open_terminal".to_owned()))
        );
        assert_eq!(
            config.keys.lookup(Context::FileList, &"Q".parse().unwrap()),
            None
        );
        assert_eq!(
            config
                .keys
                .lookup(Context::FileList, &"Ctrl+Space".parse().unwrap()),
            Some(Command::MarkRangeAbove)
        );
    }

    #[test]
    fn the_os_section_of_the_running_system_overrides_the_common_items() {
        let (config, diagnostics) = parse(EXAMPLE, Os::MacOs).unwrap();

        assert_eq!(diagnostics, Vec::new());
        assert_eq!(
            config.general.editor,
            CommandLine::parse("open -t {paths}").unwrap()
        );
        assert!(config.general.mkdir_then_enter);
        assert!(!config.general.new_text_creates_file);
        assert!(!config.general.require_marks);
        assert_eq!(
            config
                .keys
                .lookup(Context::FileList, &"Ctrl+Space".parse().unwrap()),
            None
        );
        assert_eq!(
            config
                .keys
                .lookup(Context::FileList, &"F2".parse().unwrap()),
            Some(Command::External("open_terminal".to_owned()))
        );
    }

    #[test]
    fn empty_text_is_the_default() {
        let (config, diagnostics) = parse("", Os::Windows).unwrap();

        assert_eq!(diagnostics, Vec::new());
        assert_eq!(config, Config::default_for(Os::Windows));
    }

    #[test]
    fn syntax_errors_carry_the_line_number() {
        let error = parse("[general]\nmkdir_then_enter = \n", Os::Linux).unwrap_err();

        assert_eq!(error.line, 2);
    }

    #[test]
    fn unknown_items_are_reported_and_the_rest_is_read() {
        let text = "[general]\nfoo = 1\nmkdir_then_enter = false\n[nothing]\nx = 1\n[os.bsd]\n[os.linux.appearance]\nbar = 2\n";

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert!(!config.general.mkdir_then_enter);
        assert_eq!(
            paths(&diagnostics),
            ["os.bsd", "general.foo", "appearance.bar", "nothing"]
        );
        assert!(
            diagnostics
                .iter()
                .all(|d| d.kind == DiagnosticKind::UnknownItem)
        );
    }

    #[test]
    fn invalid_values_fall_back_item_by_item() {
        let text = r##"
[appearance]
theme = 1
[appearance.font]
size = "big"
[appearance.font.log]
size = 200
[colors.light]
fg = "red"
mark = "#123"
[initial]
sort = "date"
descending = "yes"
[general]
editor = "\"unterminated"
"##;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        let default = Config::default_for(Os::Linux);
        assert_eq!(config.appearance, default.appearance);
        assert_eq!(config.colors.light.fg, default.colors.light.fg);
        assert_eq!(config.colors.light.mark, Color::new(0x11, 0x22, 0x33));
        assert_eq!(config.initial.sort, SortSpec::default());
        assert_eq!(config.general.editor, default.general.editor);
        assert_eq!(
            paths(&diagnostics),
            [
                "general.editor",
                "appearance.theme",
                "appearance.font.log.size",
                "appearance.font.size",
                "colors.light.fg",
                "initial.sort",
                "initial.descending",
            ]
        );
        assert_eq!(
            diagnostics[0].kind,
            DiagnosticKind::InvalidCommandLine(CommandLineError::UnterminatedQuote)
        );
        assert_eq!(
            diagnostics[1].kind,
            DiagnosticKind::InvalidValue {
                expected: "auto, light, or dark".to_owned()
            }
        );
    }

    #[test]
    fn keys_are_checked_against_the_commands_and_the_context() {
        let text = r#"
[commands.term]
run = "wt"
[commands.broken]
run = "x {nope}"
[keys.filelist]
"F1" = "$term"
"F2" = "$missing"
"F3" = "$broken"
"F4" = "no_such_command"
"Shift+!" = "copy"
"F5" = 3
[keys.image]
"F6" = "copy"
"Up" = "nop"
[keys.other]
"#;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(config.commands.len(), 1);
        assert_eq!(
            config
                .keys
                .lookup(Context::FileList, &"F1".parse().unwrap()),
            Some(Command::External("term".to_owned()))
        );
        assert_eq!(
            config.keys.lookup(Context::Image, &"Up".parse().unwrap()),
            None
        );
        let mut found = paths(&diagnostics);
        found.sort_unstable();
        assert_eq!(
            found,
            [
                "commands.broken.run",
                "keys.filelist",
                "keys.filelist.F2",
                "keys.filelist.F3",
                "keys.filelist.F4",
                "keys.filelist.F5",
                "keys.image.F6",
                "keys.other",
            ]
        );
        assert!(diagnostics.iter().any(|d| {
            d.kind
                == DiagnosticKind::InvalidKey {
                    key: "Shift+!".to_owned(),
                }
        }));
        assert!(diagnostics.iter().any(|d| {
            d.kind
                == DiagnosticKind::Unavailable {
                    command: "copy".to_owned(),
                }
        }));
    }

    #[test]
    fn merging_recurses_into_tables_and_replaces_everything_else() {
        let text = r#"
[[bookmarks]]
path = "/a"
[keys.filelist]
"F1" = "copy"
"F2" = "move"
[os.linux.keys.filelist]
"F2" = "delete"
[[os.linux.bookmarks]]
path = "/b"
"#;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(diagnostics, Vec::new());
        assert_eq!(config.bookmarks, [PathBuf::from("/b")]);
        assert_eq!(
            config
                .keys
                .lookup(Context::FileList, &"F1".parse().unwrap()),
            Some(Command::Copy)
        );
        assert_eq!(
            config
                .keys
                .lookup(Context::FileList, &"F2".parse().unwrap()),
            Some(Command::Delete)
        );
    }

    #[test]
    fn bookmarks_must_be_tables_with_a_path() {
        let text = "bookmarks = [1, { path = \"/a\", x = 1 }, { name = \"b\" }]\n";

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(config.bookmarks, [PathBuf::from("/a")]);
        assert_eq!(
            paths(&diagnostics),
            ["bookmarks[0]", "bookmarks[1].x", "bookmarks[2].name"]
        );
    }

    fn paths(diagnostics: &[Diagnostic]) -> Vec<&str> {
        diagnostics.iter().map(|d| d.path.as_str()).collect()
    }
}
