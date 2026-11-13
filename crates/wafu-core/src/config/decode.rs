use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::PathBuf;

use toml::{Table, Value};

use super::{
    Bookmark, Color, Colors, Config, Diagnostic, DiagnosticKind, FONT_SIZE_MAX, FONT_SIZE_MIN,
    Font, ImagePlacement, SizeFormat, SyntaxError, Theme,
};
use crate::app::Os;
use crate::command::{Command, Context};
use crate::external::CommandLine;
use crate::key::Key;

const CONTEXTS: [(&str, Context); 3] = [
    ("filelist", Context::FileList),
    ("image", Context::Image),
    ("text", Context::Text),
];

// The table is walked by hand rather than with serde, which cannot skip one
// item of the wrong type and keep the rest (serde_ignored only finds unknown
// items). A syntax error rejects the whole file and keeps the previous
// configuration; any other error drops just that item with a warning.
pub(super) fn parse(text: &str, os: Os) -> Result<(Config, Vec<Diagnostic>), SyntaxError> {
    let table: Table = toml::from_str(text).map_err(|error| SyntaxError {
        line: line_of(text, error.span()),
        message: error.message().to_owned(),
    })?;
    let mut diagnostics = Vec::new();
    let mut config = Config::default_for(os);
    let mut root = Reader::new(table, String::new(), &mut diagnostics);
    read_filelist(&mut root, &mut config);
    read_preview(&mut root, &mut config);
    read_programs(&mut root, &mut config, os);
    read_font(&mut root, &mut config);
    read_colors(&mut root, &mut config);
    read_bookmarks(&mut root, &mut config);
    read_commands(&mut root, &mut config, os);
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

    fn missing(&mut self, item: &str) {
        let path = self.path.clone();
        self.report(
            path,
            DiagnosticKind::MissingItem {
                item: item.to_owned(),
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

    fn required_string(&mut self, key: &str) -> Option<String> {
        match self.table.remove(key) {
            Some(Value::String(text)) if !text.is_empty() => Some(text),
            Some(_) => {
                self.invalid(key, "a non-empty string");
                None
            }
            None => {
                self.missing(key);
                None
            }
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
    // The message names the choices even for a value of another type, since
    // that is what the user has to write.
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

    fn opacity(&mut self, key: &str) -> Option<u8> {
        let fraction = match self.table.remove(key)? {
            Value::Float(value) => value,
            Value::Integer(0) => 0.0,
            Value::Integer(1) => 1.0,
            _ => f64::NAN,
        };
        if (0.0..=1.0).contains(&fraction) {
            // From 0 to 100, since the fraction is from 0 to 1.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Some((fraction * 100.0).round() as u8)
        } else {
            self.invalid(key, "a number from 0 to 1");
            None
        }
    }

    fn color(&mut self, key: &str) -> Option<Color> {
        self.choice(key, "a color like #rrggbb", Color::parse)
    }

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

fn read_filelist(root: &mut Reader<'_>, config: &mut Config) {
    let Some(mut filelist) = root.table("filelist") else {
        return;
    };
    let settings = &mut config.filelist;
    for (key, setting) in [
        ("mkdir_enters_dir", &mut settings.mkdir_enters_dir),
        ("new_text_creates_file", &mut settings.new_text_creates_file),
        ("extract_creates_dir", &mut settings.extract_creates_dir),
        ("require_marks", &mut settings.require_marks),
        (
            "search_matches_anywhere",
            &mut settings.search_matches_anywhere,
        ),
    ] {
        if let Some(value) = filelist.bool(key) {
            *setting = value;
        }
    }
    if let Some(format) = filelist.choice(
        "size_format",
        "plain, comma, or human",
        SizeFormat::from_name,
    ) {
        settings.size_format = format;
    }
    filelist.finish();
}

fn read_preview(root: &mut Reader<'_>, config: &mut Config) {
    let Some(mut preview) = root.table("preview") else {
        return;
    };
    if let Some(mut text) = preview.table("text") {
        if let Some(value) = text.bool("hide_log") {
            config.preview.text.hide_log = value;
        }
        text.finish();
    }
    if let Some(mut image) = preview.table("image") {
        let settings = &mut config.preview.image;
        for (key, setting) in [
            ("from_left", &mut settings.from_left),
            ("from_right", &mut settings.from_right),
        ] {
            if let Some(placement) =
                image.choice(key, "other_pane or both_panes", ImagePlacement::from_name)
            {
                *setting = placement;
            }
        }
        if let Some(value) = image.bool("hide_log") {
            settings.hide_log = value;
        }
        image.finish();
    }
    preview.finish();
}

fn read_programs(root: &mut Reader<'_>, config: &mut Config, os: Os) {
    let Some(mut programs) = root.table("programs") else {
        return;
    };
    if let Some(text) = programs.string("editor") {
        match CommandLine::parse(&text, os) {
            Ok(editor) => config.programs.editor = editor,
            Err(error) => {
                let path = programs.path_of("editor");
                programs.report(path, DiagnosticKind::InvalidCommandLine(error));
            }
        }
    }
    if let Some(value) = programs.bool("use_login_shell_env") {
        config.programs.use_login_shell_env = value;
    }
    programs.finish();
}

fn read_font(root: &mut Reader<'_>, config: &mut Config) {
    let Some(mut font) = root.table("font") else {
        return;
    };
    let fonts = &mut config.font;
    for (key, area) in [
        ("path", &mut fonts.path),
        ("filelist", &mut fonts.filelist),
        ("info", &mut fonts.info),
        ("log", &mut fonts.log),
        ("status", &mut fonts.status),
        ("preview", &mut fonts.preview),
        ("popup", &mut fonts.popup),
    ] {
        if let Some(mut reader) = font.table(key) {
            read_font_items(&mut reader, area);
            reader.finish();
        }
    }
    read_font_items(&mut font, &mut fonts.default);
    font.finish();
}

fn read_font_items(reader: &mut Reader<'_>, font: &mut Font) {
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
    if let Some(theme) = colors.choice("theme", "auto, light, or dark", Theme::from_name) {
        config.colors.theme = theme;
    }
    if let Some(opacity) = colors.opacity("hidden_opacity") {
        config.colors.hidden_opacity = opacity;
    }
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
            line_number,
            mark,
            cursor,
            cursor_inactive,
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
            ("line_number", line_number),
            ("mark", mark),
            ("cursor", cursor),
            ("cursor_inactive", cursor_inactive),
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

fn read_bookmarks(root: &mut Reader<'_>, config: &mut Config) {
    let Some(value) = root.table.remove("bookmarks") else {
        return;
    };
    let Value::Array(items) = value else {
        root.invalid("bookmarks", "an array of tables");
        return;
    };
    let mut users: BTreeMap<char, Vec<String>> = BTreeMap::new();
    for (index, item) in items.into_iter().enumerate() {
        let item_key = format!("bookmarks[{index}]");
        let Value::Table(table) = item else {
            root.invalid(&item_key, "a table");
            continue;
        };
        let item_path = root.path_of(&item_key);
        let mut reader = Reader::new(table, item_path.clone(), root.diagnostics);
        let name = reader.required_string("name");
        let path = reader.required_string("path");
        let key = reader.choice("key", "a digit or a letter", bookmark_key);
        if let (Some(name), Some(path)) = (name, path) {
            if let Some(key) = key {
                users.entry(key).or_default().push(item_path);
            }
            config.bookmarks.push(Bookmark {
                name,
                path: PathBuf::from(path),
                key,
            });
        }
        reader.finish();
    }
    for (key, bookmarks) in users {
        if bookmarks.len() > 1 {
            for bookmark in &mut config.bookmarks {
                if bookmark.key == Some(key) {
                    bookmark.key = None;
                }
            }
            let path = root.path_of("bookmarks");
            root.report(
                path,
                DiagnosticKind::DuplicateBookmarkKey { key, bookmarks },
            );
        }
    }
}

// Only what the popup's own keys (the arrows, Enter, Esc) can never be, and
// what reads the same on every keyboard layout.
fn bookmark_key(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphanumeric() => Some(c.to_ascii_lowercase()),
        _ => None,
    }
}

fn read_commands(root: &mut Reader<'_>, config: &mut Config, os: Os) {
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
        if let Some(run) = reader.required_string("run") {
            match CommandLine::parse(&run, os) {
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
        let mut spellings: HashMap<Key, Vec<String>> = HashMap::new();
        let mut entries = Vec::new();
        for (text, value) in reader.entries() {
            let Ok(key) = text.parse::<Key>() else {
                let path = reader.path.clone();
                reader.report(path, DiagnosticKind::InvalidKey { key: text });
                continue;
            };
            spellings.entry(key).or_default().push(text.clone());
            entries.push((text, key, value));
        }
        for (text, key, value) in entries {
            let same = &spellings[&key];
            if same.len() > 1 {
                if same[0] == text {
                    let path = reader.path.clone();
                    reader.report(
                        path,
                        DiagnosticKind::DuplicateKey {
                            key: key.to_string(),
                            spellings: same.clone(),
                        },
                    );
                }
                continue;
            }
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

// Tables merge item by item and key tables by normalized key, so that an
// overlay changes only what it writes; arrays and scalars are replaced.
// Built only for the tests while no configuration is read in layers, since
// unused code fails the build.
#[cfg(test)]
pub(crate) fn merge(base: &mut Table, mut overlay: Table) {
    if let Some(Value::Table(overlay_keys)) = overlay.get_mut("keys")
        && let Some(Value::Table(base_keys)) = base.get_mut("keys")
    {
        for (context, value) in std::mem::take(overlay_keys) {
            match (base_keys.get_mut(&context), value) {
                (Some(Value::Table(below)), Value::Table(above)) => merge_key_table(below, above),
                (_, value) => {
                    base_keys.insert(context, value);
                }
            }
        }
    }
    merge_tables(base, overlay);
}

#[cfg(test)]
fn merge_key_table(base: &mut Table, overlay: Table) {
    let overlaid: Vec<Key> = overlay
        .keys()
        .filter_map(|spelling| spelling.parse().ok())
        .collect();
    base.retain(|spelling, _| {
        spelling
            .parse::<Key>()
            .ok()
            .is_none_or(|key| !overlaid.contains(&key))
    });
    base.extend(overlay);
}

#[cfg(test)]
fn merge_tables(base: &mut Table, overlay: Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(Value::Table(below)), Value::Table(above)) => merge_tables(below, above),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

fn line_of(text: &str, span: Option<Range<usize>>) -> usize {
    let start = span.map_or(0, |span| span.start).min(text.len());
    text[..start].matches('\n').count() + 1
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::*;
    use crate::config::{ImagePreviewConfig, PreviewConfig, TextPreviewConfig};
    use crate::external::{CommandLineError, Expansion};

    const EXAMPLE: &str = r##"
[filelist]
mkdir_enters_dir = true
new_text_creates_file = false
extract_creates_dir = false
require_marks = false
size_format = "human"          # plain / comma / human

[preview.text]
hide_log = true

[preview.image]
from_left = "both_panes"
from_right = "other_pane"
hide_log = true

[programs]
editor = "notepad {paths}"

[font]                         # the default
family = "..."
size = 12

[font.filelist]                # per area (path, info, log, status, preview, and popup too)
family = "..."

[colors]
theme = "auto"                 # auto / light / dark

[colors.light]
fg = "#000000"
bg = "#ffffff"
line_number = "#808080"
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

[[bookmarks]]
name = "Downloads"
path = "~/Downloads"

[commands.open_terminal]
run = "wt -d {dir}"

[keys.filelist]
"F2" = "$open_terminal"
"Q" = "nop"

[keys.image]
[keys.text]
"##;

    #[test]
    fn a_full_example_reads_without_warnings() {
        let (config, diagnostics) = parse(EXAMPLE, Os::Linux).unwrap();

        assert_eq!(diagnostics, Vec::new());
        assert!(!config.filelist.new_text_creates_file);
        assert_eq!(config.filelist.size_format, SizeFormat::Human);
        assert_eq!(
            config.programs.editor,
            CommandLine::parse("notepad {paths}", Os::Linux).unwrap()
        );
        assert_eq!(
            config.preview,
            PreviewConfig {
                text: TextPreviewConfig { hide_log: true },
                image: ImagePreviewConfig {
                    from_left: ImagePlacement::BothPanes,
                    from_right: ImagePlacement::OtherPane,
                    hide_log: true,
                },
            }
        );
        assert_eq!(config.font.default.family, Some("...".to_owned()));
        assert_eq!(config.font.filelist.size, None);
        assert_eq!(
            config.colors.light.line_number,
            Color::new(0x80, 0x80, 0x80)
        );
        assert_eq!(config.colors.light.dir, Color::new(0x00, 0x00, 0xff));
        assert_eq!(config.bookmarks, [bookmark("Downloads", "~/Downloads")]);
        assert_eq!(
            config.commands["open_terminal"],
            CommandLine::parse("wt -d {dir}", Os::Linux).unwrap()
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
    fn syntax_errors_carry_the_line_number() {
        let error = parse("[filelist]\nmkdir_enters_dir = \n", Os::Linux).unwrap_err();

        assert_eq!(error.line, 2);
    }

    #[test]
    fn unknown_items_are_reported_and_the_rest_is_read() {
        let text = "[filelist]\nfoo = 1\nmkdir_enters_dir = false\n[colors.light]\nbar = 2\n[nothing]\nx = 1\n";

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert!(!config.filelist.mkdir_enters_dir);
        assert_eq!(
            paths(&diagnostics),
            ["filelist.foo", "colors.light.bar", "nothing"]
        );
        assert!(
            diagnostics
                .iter()
                .all(|d| d.kind == DiagnosticKind::UnknownItem)
        );
    }

    #[test]
    fn command_lines_are_split_by_the_rules_of_the_given_os() {
        let (config, diagnostics) = parse(
            "[programs]\neditor = \"x 'a b'\"\n[commands.x]\nrun = \"x 'a b'\"\n",
            Os::Windows,
        )
        .unwrap();

        assert_eq!(diagnostics, Vec::new());
        assert_eq!(
            config.programs.editor.expand(&Expansion::default()),
            ["'a", "b'"].map(OsString::from)
        );
        assert_eq!(
            config.commands["x"].expand(&Expansion::default()),
            ["'a", "b'"].map(OsString::from)
        );
    }

    #[test]
    fn hidden_opacity_is_a_fraction_from_zero_to_one() {
        let read = |value: &str| {
            let text = format!("[colors]\nhidden_opacity = {value}\n");
            let (config, diagnostics) = parse(&text, Os::Linux).unwrap();
            (config.colors.hidden_opacity, paths(&diagnostics).len())
        };

        assert_eq!(read("0.0"), (0, 0));
        assert_eq!(read("0.255"), (26, 0));
        assert_eq!(read("1.0"), (100, 0));
        assert_eq!(read("0"), (0, 0));
        assert_eq!(read("1"), (100, 0));
        assert_eq!(read("1.01"), (70, 1));
        assert_eq!(read("-0.1"), (70, 1));
        assert_eq!(read("2"), (70, 1));
        assert_eq!(read("nan"), (70, 1));
        assert_eq!(read("\"0.5\""), (70, 1));
    }

    #[test]
    fn invalid_values_fall_back_item_by_item() {
        let text = r##"
[filelist]
size_format = "bogus"
[preview.image]
from_left = "full"
from_right = true
[programs]
editor = "\"unterminated"
[font]
size = "big"
[font.log]
size = 200
[colors]
theme = 1
[colors.light]
fg = "red"
mark = "#123"
"##;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        let default = Config::default_for(Os::Linux);
        assert_eq!(config.filelist, default.filelist);
        assert_eq!(config.preview, default.preview);
        assert_eq!(config.programs, default.programs);
        assert_eq!(config.font, default.font);
        assert_eq!(config.colors.theme, default.colors.theme);
        assert_eq!(config.colors.light.fg, default.colors.light.fg);
        assert_eq!(config.colors.light.mark, Color::new(0x11, 0x22, 0x33));
        assert_eq!(
            paths(&diagnostics),
            [
                "filelist.size_format",
                "preview.image.from_left",
                "preview.image.from_right",
                "programs.editor",
                "font.log.size",
                "font.size",
                "colors.theme",
                "colors.light.fg",
            ]
        );
        assert_eq!(
            diagnostics[1].kind,
            DiagnosticKind::InvalidValue {
                expected: "other_pane or both_panes".to_owned()
            }
        );
        assert_eq!(diagnostics[2].kind, diagnostics[1].kind);
        assert_eq!(
            diagnostics[3].kind,
            DiagnosticKind::InvalidCommandLine(CommandLineError::UnterminatedQuoteOrBackslash)
        );
        assert_eq!(
            diagnostics[6].kind,
            DiagnosticKind::InvalidValue {
                expected: "auto, light, or dark".to_owned()
            }
        );
    }

    #[test]
    fn each_font_area_is_read_into_its_own_font() {
        let areas = [
            "path", "filelist", "info", "log", "status", "preview", "popup",
        ];
        let text = areas
            .map(|area| format!("[font.{area}]\nfamily = \"{area}\"\n"))
            .concat();

        let (config, diagnostics) = parse(&text, Os::Linux).unwrap();

        assert_eq!(diagnostics, Vec::new());
        let fonts = &config.font;
        assert_eq!(
            [
                &fonts.path,
                &fonts.filelist,
                &fonts.info,
                &fonts.log,
                &fonts.status,
                &fonts.preview,
                &fonts.popup,
            ]
            .map(|font| font.family.as_deref()),
            areas.map(Some)
        );
    }

    #[test]
    fn keys_are_checked_against_the_commands_and_the_context() {
        let text = r#"
[commands.term]
run = "wt"
[commands.broken]
run = "x 'unterminated"
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
    fn spellings_of_one_key_in_a_table_are_all_ignored() {
        let text = r#"
[keys.filelist]
"a" = "copy"
"A" = "move"
"ctrl+b" = "copy"
"Ctrl+B" = "move"
"Ctrl+b" = "delete"
"F1" = "copy"
"#;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        let default = Config::default_for(Os::Linux);
        for key in ["a", "A", "Ctrl+B"] {
            let key: Key = key.parse().unwrap();
            assert_eq!(
                config.keys.lookup(Context::FileList, &key),
                default.keys.lookup(Context::FileList, &key)
            );
        }
        assert_eq!(
            config
                .keys
                .lookup(Context::FileList, &"F1".parse().unwrap()),
            Some(Command::Copy)
        );
        assert_eq!(paths(&diagnostics), ["keys.filelist", "keys.filelist"]);
        assert_eq!(
            diagnostics[0].kind,
            DiagnosticKind::DuplicateKey {
                key: "A".to_owned(),
                spellings: vec!["A".to_owned(), "a".to_owned()],
            }
        );
        assert_eq!(
            diagnostics[1].kind,
            DiagnosticKind::DuplicateKey {
                key: "Ctrl+B".to_owned(),
                spellings: vec![
                    "Ctrl+B".to_owned(),
                    "Ctrl+b".to_owned(),
                    "ctrl+b".to_owned()
                ],
            }
        );
    }

    #[test]
    fn spellings_of_one_key_in_different_tables_do_not_collide() {
        let text = r#"
[keys.filelist]
"a" = "copy"
[keys.image]
"A" = "nop"
"#;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(diagnostics, Vec::new());
        let key: Key = "a".parse().unwrap();
        assert_eq!(
            config.keys.lookup(Context::FileList, &key),
            Some(Command::Copy)
        );
        assert_eq!(config.keys.lookup(Context::Image, &key), None);
    }

    #[test]
    fn bookmarks_must_be_tables_with_a_name_and_a_path() {
        let text = "bookmarks = [1, { name = \"a\", path = \"/a\", x = 1 }, { name = \"b\" }, { path = \"/c\" }, { name = 1, path = \"/d\" }, { name = \"\", path = \"/e\" }, { name = \"f\", path = \"\" }]\n";

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(config.bookmarks, [bookmark("a", "/a")]);
        assert_eq!(
            paths(&diagnostics),
            [
                "bookmarks[0]",
                "bookmarks[1].x",
                "bookmarks[2]",
                "bookmarks[3]",
                "bookmarks[4].name",
                "bookmarks[5].name",
                "bookmarks[6].path"
            ]
        );
        assert_eq!(
            diagnostics[2].kind,
            DiagnosticKind::MissingItem {
                item: "path".to_owned()
            }
        );
        assert_eq!(
            diagnostics[3].kind,
            DiagnosticKind::MissingItem {
                item: "name".to_owned()
            }
        );
        assert_eq!(
            diagnostics[5].kind,
            DiagnosticKind::InvalidValue {
                expected: "a non-empty string".to_owned()
            }
        );
        assert_eq!(
            diagnostics[6].to_string(),
            "Config: invalid value for bookmarks[6].path (expected a non-empty string)"
        );
    }

    #[test]
    fn bookmark_keys_are_single_digits_or_letters_in_either_case() {
        let text = r#"
bookmarks = [
    { name = "a", path = "/a", key = "D" },
    { name = "b", path = "/b", key = "7" },
    { name = "c", path = "/c" },
    { name = "d", path = "/d", key = "ab" },
    { name = "e", path = "/e", key = "" },
    { name = "f", path = "/f", key = "@" },
    { name = "g", path = "/g", key = 1 },
    { name = "h", path = "/h", key = "\u00e9" },
]
"#;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(
            config
                .bookmarks
                .iter()
                .map(|bookmark| bookmark.key)
                .collect::<Vec<_>>(),
            [Some('d'), Some('7'), None, None, None, None, None, None]
        );
        assert_eq!(
            paths(&diagnostics),
            [
                "bookmarks[3].key",
                "bookmarks[4].key",
                "bookmarks[5].key",
                "bookmarks[6].key",
                "bookmarks[7].key"
            ]
        );
        assert_eq!(
            diagnostics[0].to_string(),
            "Config: invalid value for bookmarks[3].key (expected a digit or a letter)"
        );
    }

    #[test]
    fn a_key_shared_by_bookmarks_is_taken_from_all_of_them() {
        let text = r#"
bookmarks = [
    { name = "a", path = "/a", key = "d" },
    { name = "b", path = "/b", key = "x" },
    { name = "c", key = "x" },
    { name = "d", path = "/d", key = "D" },
    { name = "e", path = "/e", key = "1" },
    { name = "f", path = "/f", key = "1" },
    { name = "g", path = "/g", key = "d" },
]
"#;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(
            config
                .bookmarks
                .iter()
                .map(|bookmark| (bookmark.name.as_str(), bookmark.key))
                .collect::<Vec<_>>(),
            [
                ("a", None),
                ("b", Some('x')),
                ("d", None),
                ("e", None),
                ("f", None),
                ("g", None)
            ]
        );
        assert_eq!(
            diagnostics[1..],
            [
                Diagnostic {
                    path: "bookmarks".to_owned(),
                    kind: DiagnosticKind::DuplicateBookmarkKey {
                        key: '1',
                        bookmarks: vec!["bookmarks[4]".to_owned(), "bookmarks[5]".to_owned()],
                    },
                },
                Diagnostic {
                    path: "bookmarks".to_owned(),
                    kind: DiagnosticKind::DuplicateBookmarkKey {
                        key: 'd',
                        bookmarks: vec![
                            "bookmarks[0]".to_owned(),
                            "bookmarks[3]".to_owned(),
                            "bookmarks[6]".to_owned()
                        ],
                    },
                }
            ]
        );
    }

    #[test]
    fn definitions_without_a_required_item_are_dropped_alone() {
        let text = r#"
[[bookmarks]]
name = "none"
[[bookmarks]]
name = "a"
path = "/a"
[commands.empty]
run = ""
[commands.norun]
[commands.term]
run = "wt"
[keys.filelist]
"F1" = "$norun"
"F2" = "$term"
"#;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(config.bookmarks, [bookmark("a", "/a")]);
        assert_eq!(
            config
                .commands
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["term"]
        );
        assert_eq!(
            paths(&diagnostics),
            [
                "bookmarks[0]",
                "commands.empty.run",
                "commands.norun",
                "keys.filelist.F1"
            ]
        );
        assert_eq!(
            diagnostics[0].kind,
            DiagnosticKind::MissingItem {
                item: "path".to_owned()
            }
        );
        assert_eq!(
            diagnostics[1].kind,
            DiagnosticKind::InvalidValue {
                expected: "a non-empty string".to_owned()
            }
        );
        assert_eq!(
            diagnostics[2].kind,
            DiagnosticKind::MissingItem {
                item: "run".to_owned()
            }
        );
        assert_eq!(
            diagnostics[0].to_string(),
            "Config: missing path in bookmarks[0]"
        );
    }

    #[test]
    fn merging_overlays_key_tables_by_normalized_key() {
        let mut base = table("[keys.filelist]\n\"a\" = \"copy\"\n\"F1\" = \"copy\"\n");

        merge(&mut base, table("[keys.filelist]\n\"A\" = \"move\"\n"));

        assert_eq!(
            base,
            table("[keys.filelist]\n\"A\" = \"move\"\n\"F1\" = \"copy\"\n")
        );
    }

    #[test]
    fn merging_recurses_into_tables_and_replaces_everything_else() {
        let mut base = table(
            "[[bookmarks]]\nname = \"a\"\npath = \"/a\"\n[filelist]\nrequire_marks = false\nsize_format = \"comma\"\n[font.log]\nsize = 10\n",
        );

        merge(
            &mut base,
            table(
                "[[bookmarks]]\nname = \"b\"\npath = \"/b\"\n[filelist]\nsize_format = \"human\"\n[font]\nlog = 1\n",
            ),
        );

        assert_eq!(
            base,
            table(
                "[[bookmarks]]\nname = \"b\"\npath = \"/b\"\n[filelist]\nrequire_marks = false\nsize_format = \"human\"\n[font]\nlog = 1\n",
            )
        );
    }

    fn paths(diagnostics: &[Diagnostic]) -> Vec<&str> {
        diagnostics.iter().map(|d| d.path.as_str()).collect()
    }

    fn table(text: &str) -> Table {
        toml::from_str(text).unwrap()
    }

    fn bookmark(name: &str, path: &str) -> Bookmark {
        Bookmark {
            name: name.to_owned(),
            path: PathBuf::from(path),
            key: None,
        }
    }
}
