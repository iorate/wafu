use std::collections::{BTreeMap, HashMap};
use std::ops::Range;
use std::path::{Path, PathBuf};

use toml::{Table, Value};

use super::settings::{
    COLORS, COLORS_CONFIG, FILELIST, FONT, IMAGE_PREVIEW, Kind, LOG, PROGRAMS, Rgb, Setting,
    TEXT_PREVIEW,
};
use super::{Bookmark, Config, Diagnostic, DiagnosticKind, OpenAlternativeConfig, SyntaxError};
use crate::app::Os;
use crate::command::{Command, Context};
use crate::external::CommandLine;
use crate::key::Key;

const PREVIEW_CONTEXTS: [(&str, Context); 2] = [("image", Context::Image), ("text", Context::Text)];

const EXTENSION_TABLES: [&[&str]; 3] = [
    &["colors", "light", "extensions"],
    &["colors", "dark", "extensions"],
    &["open_alternative", "extensions"],
];

const KEY_TABLES: [&[&str]; 3] = [
    &["keys", "filelist"],
    &["keys", "preview", "image"],
    &["keys", "preview", "text"],
];

// The table is walked by hand rather than with serde, which cannot skip one
// item of the wrong type and keep the rest (serde_ignored only finds unknown
// items). A syntax error rejects every file and keeps the previous
// configuration; any other error drops just that item with a warning.
pub(super) fn parse_layers<'a>(
    files: impl IntoIterator<Item = (&'a Path, &'a str)>,
    os: Os,
) -> Result<(Config, Vec<Diagnostic>), (usize, SyntaxError)> {
    let mut merged = Table::new();
    let mut diagnostics = Vec::new();
    let mut bookmarks_dir = Path::new("");
    for (index, (file, text)) in files.into_iter().enumerate() {
        let mut table = parse_table(text).map_err(|error| (index, error))?;
        take_extends(&mut table, &mut diagnostics);
        // Arrays replace wholesale in the merge, so every bookmark comes from
        // the last file that has them.
        if table.contains_key("bookmarks") {
            bookmarks_dir = file.parent().unwrap_or(file);
        }
        merge(&mut merged, table);
    }
    let mut config = Config::default_for(os);
    let mut root = Reader::new(merged, String::new(), &mut diagnostics);
    read_filelist(&mut root, &mut config, os);
    read_preview(&mut root, &mut config, os);
    read_log(&mut root, &mut config, os);
    read_programs(&mut root, &mut config, os);
    read_font(&mut root, &mut config, os);
    read_colors(&mut root, &mut config, os);
    read_bookmarks(&mut root, &mut config, bookmarks_dir);
    read_commands(&mut root, &mut config, os);
    read_open_alternative(&mut root, &mut config, os);
    read_keys(&mut root, &mut config);
    root.finish();
    Ok((config, diagnostics))
}

// A file whose text is not TOML names no bases; its syntax error is reported
// when the files are parsed.
pub(super) fn extends(text: &str) -> Vec<String> {
    parse_table(text).map_or_else(
        |_| Vec::new(),
        |mut table| take_extends(&mut table, &mut Vec::new()),
    )
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

    fn settings<T>(&mut self, settings: &[Setting<T>], target: &mut T, os: Os) {
        for setting in settings {
            if setting.empty_unsets && self.take_unset(setting.key) {
                continue;
            }
            if let Some(value) = self.table.remove(setting.key) {
                let read = (setting.read)(target, value, os);
                self.checked(setting.key, read);
            }
        }
    }

    fn checked<T>(&mut self, key: &str, read: Result<T, DiagnosticKind>) -> Option<T> {
        match read {
            Ok(value) => Some(value),
            Err(kind) => {
                let path = self.path_of(key);
                self.report(path, kind);
                None
            }
        }
    }

    // TOML has no null, so an empty string stands for an item that no file
    // has written: it is how a file takes back what a file it extends set.
    // Nothing takes an empty string as a value of its own. Items of an array
    // are not read this way, since an array replaces the one below it whole.
    fn take_unset(&mut self, key: &str) -> bool {
        let unset = matches!(self.table.get(key), Some(Value::String(text)) if text.is_empty());
        if unset {
            self.table.remove(key);
        }
        unset
    }

    fn string(&mut self, key: &str) -> Option<String> {
        if self.take_unset(key) {
            return None;
        }
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

    // Reads a string and maps it; anything the mapping rejects is invalid.
    // The message names the choices even for a value of another type, since
    // that is what the user has to write.
    fn parsed<T>(
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

    fn command_line(&mut self, key: &str, text: &str, os: Os) -> Option<CommandLine> {
        let line = CommandLine::parse(text, os).map_err(DiagnosticKind::InvalidCommandLine);
        self.checked(key, line)
    }

    fn file_line(&mut self, key: &str, text: &str, os: Os) -> Option<CommandLine> {
        let line =
            CommandLine::parse_for_file(text, os).map_err(DiagnosticKind::InvalidCommandLine);
        self.checked(key, line)
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

fn read_filelist(root: &mut Reader<'_>, config: &mut Config, os: Os) {
    if let Some(mut filelist) = root.table("filelist") {
        filelist.settings(FILELIST, &mut config.filelist, os);
        filelist.finish();
    }
}

fn read_preview(root: &mut Reader<'_>, config: &mut Config, os: Os) {
    let Some(mut preview) = root.table("preview") else {
        return;
    };
    if let Some(mut text) = preview.table("text") {
        text.settings(TEXT_PREVIEW, &mut config.preview.text, os);
        text.finish();
    }
    if let Some(mut image) = preview.table("image") {
        image.settings(IMAGE_PREVIEW, &mut config.preview.image, os);
        image.finish();
    }
    preview.finish();
}

fn read_log(root: &mut Reader<'_>, config: &mut Config, os: Os) {
    if let Some(mut log) = root.table("log") {
        log.settings(LOG, &mut config.log, os);
        log.finish();
    }
}

fn read_programs(root: &mut Reader<'_>, config: &mut Config, os: Os) {
    if let Some(mut programs) = root.table("programs") {
        programs.settings(PROGRAMS, &mut config.programs, os);
        programs.finish();
    }
}

fn read_font(root: &mut Reader<'_>, config: &mut Config, os: Os) {
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
            reader.settings(FONT, area, os);
            reader.finish();
        }
    }
    font.settings(FONT, &mut fonts.default, os);
    font.finish();
}

fn read_colors(root: &mut Reader<'_>, config: &mut Config, os: Os) {
    let Some(mut colors) = root.table("colors") else {
        return;
    };
    colors.settings(COLORS_CONFIG, &mut config.colors, os);
    for (key, set) in [
        ("light", &mut config.colors.light),
        ("dark", &mut config.colors.dark),
    ] {
        let Some(mut reader) = colors.table(key) else {
            continue;
        };
        if let Some(mut table) = reader.table("extensions") {
            read_extensions(&mut table, &mut set.extensions, |table, key, value| {
                table.checked(key, Rgb.parse(value, os))
            });
            table.finish();
        }
        reader.settings(COLORS, set, os);
        reader.finish();
    }
    colors.finish();
}

fn read_extensions<T>(
    reader: &mut Reader<'_>,
    extensions: &mut BTreeMap<String, T>,
    mut read: impl FnMut(&mut Reader<'_>, &str, Value) -> Option<T>,
) {
    let mut spellings: HashMap<String, Vec<String>> = HashMap::new();
    let mut entries = Vec::new();
    for (text, value) in reader.entries() {
        let extension = text.to_ascii_lowercase();
        if extension.is_empty() || extension.contains('.') {
            let path = reader.path.clone();
            reader.report(path, DiagnosticKind::InvalidKey { key: text });
            continue;
        }
        spellings
            .entry(extension.clone())
            .or_default()
            .push(text.clone());
        entries.push((text, extension, value));
    }
    for (text, extension, value) in entries {
        let same = &spellings[&extension];
        if same.len() > 1 {
            if same[0] == text {
                let path = reader.path.clone();
                reader.report(
                    path,
                    DiagnosticKind::DuplicateKey {
                        key: extension,
                        spellings: same.clone(),
                    },
                );
            }
            continue;
        }
        if matches!(&value, Value::String(text) if text.is_empty()) {
            continue;
        }
        if let Some(value) = read(reader, &text, value) {
            extensions.insert(extension, value);
        }
    }
}

fn read_bookmarks(root: &mut Reader<'_>, config: &mut Config, dir: &Path) {
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
        let key = reader.parsed("key", "a digit or a letter", bookmark_key);
        if let (Some(name), Some(path)) = (name, path) {
            if let Some(key) = key {
                users.entry(key).or_default().push(item_path);
            }
            config.bookmarks.push(Bookmark {
                name,
                path: PathBuf::from(path),
                dir: dir.to_path_buf(),
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
        let line = match value {
            Value::String(text) if text.is_empty() => None,
            Value::String(text) => commands.command_line(&name, &text, os),
            Value::Table(table) => {
                let path = commands.path_of(&name);
                let mut reader = Reader::new(table, path, commands.diagnostics);
                let line = if reader.take_unset("command_line") {
                    None
                } else {
                    reader
                        .required_string("command_line")
                        .and_then(|text| reader.command_line("command_line", &text, os))
                };
                reader.finish();
                line
            }
            _ => {
                commands.invalid(&name, "a string or a table");
                None
            }
        };
        if let Some(line) = line {
            config.commands.insert(name, line);
        }
    }
    commands.finish();
}

fn read_open_alternative(root: &mut Reader<'_>, config: &mut Config, os: Os) {
    let Some(mut table) = root.table("open_alternative") else {
        return;
    };
    let OpenAlternativeConfig { dir, extensions } = &mut config.open_alternative;
    if let Some(text) = table.string("dir")
        && let Some(line) = table.file_line("dir", &text, os)
    {
        *dir = Some(line);
    }
    if let Some(mut reader) = table.table("extensions") {
        read_extensions(&mut reader, extensions, |reader, key, value| {
            let Value::String(text) = value else {
                reader.invalid(key, "a string");
                return None;
            };
            reader.file_line(key, &text, os)
        });
        reader.finish();
    }
    table.finish();
}

fn read_keys(root: &mut Reader<'_>, config: &mut Config) {
    let Some(mut keys) = root.table("keys") else {
        return;
    };
    if let Some(mut reader) = keys.table("filelist") {
        read_key_table(&mut reader, Context::FileList, config);
        reader.finish();
    }
    if let Some(mut preview) = keys.table("preview") {
        for (name, context) in PREVIEW_CONTEXTS {
            if let Some(mut reader) = preview.table(name) {
                read_key_table(&mut reader, context, config);
                reader.finish();
            }
        }
        preview.finish();
    }
    keys.finish();
}

fn read_key_table(reader: &mut Reader<'_>, context: Context, config: &mut Config) {
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
        if name.is_empty() {
            continue;
        }
        let path = reader.path_of(&text);
        let command = match Command::from_name(&name) {
            Some(Command::External(external)) if !config.commands.contains_key(&external) => None,
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
}

// Key tables and tables keyed by extension merge by normalized key, so that an
// overlay's `"a"` replaces a `"A"` below it rather than both being reported as
// the same key twice.
fn merge(base: &mut Table, overlay: Table) {
    for path in EXTENSION_TABLES {
        if let Some((above, below)) = tables_at(&overlay, base, path) {
            below.retain(|spelling, _| {
                !above
                    .keys()
                    .any(|overlaid| overlaid.eq_ignore_ascii_case(spelling))
            });
        }
    }
    for path in KEY_TABLES {
        if let Some((above, below)) = tables_at(&overlay, base, path) {
            let overlaid: Vec<Key> = above
                .keys()
                .filter_map(|spelling| spelling.parse().ok())
                .collect();
            below.retain(|spelling, _| {
                spelling
                    .parse::<Key>()
                    .ok()
                    .is_none_or(|key| !overlaid.contains(&key))
            });
        }
    }
    merge_tables(base, overlay);
}

fn tables_at<'a, 'b>(
    overlay: &'a Table,
    base: &'b mut Table,
    path: &[&str],
) -> Option<(&'a Table, &'b mut Table)> {
    let above = path
        .iter()
        .try_fold(overlay, |table, key| table.get(*key)?.as_table())?;
    let below = path
        .iter()
        .try_fold(base, |table, key| table.get_mut(*key)?.as_table_mut())?;
    Some((above, below))
}

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

fn parse_table(text: &str) -> Result<Table, SyntaxError> {
    toml::from_str(text).map_err(|error| SyntaxError {
        line: line_of(text, error.span()),
        message: error.message().to_owned(),
    })
}

// Taken out of every file before the files merge: each file names its own
// bases, so the key never carries over to the file that extends it.
fn take_extends(table: &mut Table, diagnostics: &mut Vec<Diagnostic>) -> Vec<String> {
    let mut invalid = |path: String, expected: &str| {
        diagnostics.push(Diagnostic {
            path,
            kind: DiagnosticKind::InvalidValue {
                expected: expected.to_owned(),
            },
        });
    };
    match table.remove("extends") {
        None => Vec::new(),
        // Left unwritten, as an empty string is for other string items. Read
        // as a path, it would name the directory of the file itself.
        Some(Value::String(path)) if path.is_empty() => Vec::new(),
        Some(Value::String(path)) => vec![path],
        Some(Value::Array(items)) => items
            .into_iter()
            .enumerate()
            .filter_map(|(index, item)| match item {
                Value::String(path) if !path.is_empty() => Some(path),
                _ => {
                    invalid(format!("extends[{index}]"), "a non-empty string");
                    None
                }
            })
            .collect(),
        Some(_) => {
            invalid("extends".to_owned(), "a string or an array of strings");
            Vec::new()
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
    use crate::config::{
        Color, ImagePlacement, ImagePreviewConfig, PreviewConfig, SearchMatch, SizeFormat,
        TextPreviewConfig, TimeFormat,
    };
    use crate::external::{CommandLineError, Expansion};

    const FILE: &str = "/c/config.toml";
    const EXAMPLE: &str = r##"
[filelist]
new_dir_goes_to_dir = true
new_text_creates_file = false
extract_creates_dir = false
require_marks = false
search_match = "start"         # anywhere / start
size_format = "human"          # plain / comma / human
time_shows_full_year = false
time_shows_seconds = false

[preview.text]
hide_log = true

[preview.image]
placement_from_left = "both_panes"
placement_from_right = "other_pane"
hide_log = true

[programs]
editor = "notepad {path}"

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

[colors.light.extensions]
zip = "#a02020"

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
command_line = "wt -d {dir}"

[open_alternative]
dir = "code {path}"

[open_alternative.extensions]
pdf = "sumatrapdf {path}"

[keys.filelist]
"F2" = "$open_terminal"
"Q" = "nop"

[keys.preview.image]
[keys.preview.text]
"##;

    #[test]
    fn a_full_example_reads_without_warnings() {
        let (config, diagnostics) = parse(EXAMPLE, Os::Linux).unwrap();

        assert_eq!(diagnostics, Vec::new());
        assert!(!config.filelist.new_text_creates_file);
        assert_eq!(config.filelist.search_match, SearchMatch::Start);
        assert_eq!(config.filelist.size_format, SizeFormat::Human);
        assert_eq!(
            config.filelist.time_format,
            TimeFormat {
                full_year: false,
                seconds: false,
            }
        );
        assert_eq!(
            config.programs.editor,
            CommandLine::parse_for_file("notepad {path}", Os::Linux).unwrap()
        );
        assert_eq!(
            config.preview,
            PreviewConfig {
                text: TextPreviewConfig {
                    hide_log: true,
                    max_bytes: 16 * 1024 * 1024,
                },
                image: ImagePreviewConfig {
                    placement_from_left: ImagePlacement::BothPanes,
                    placement_from_right: ImagePlacement::OtherPane,
                    hide_log: true,
                    max_bytes: 64 * 1024 * 1024,
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
        assert_eq!(
            config.colors.light.extensions,
            BTreeMap::from([("zip".to_owned(), Color::new(0xa0, 0x20, 0x20))])
        );
        assert_eq!(config.bookmarks, [bookmark("Downloads", "~/Downloads")]);
        assert_eq!(
            config.commands["open_terminal"],
            CommandLine::parse("wt -d {dir}", Os::Linux).unwrap()
        );
        assert_eq!(
            config.open_alternative,
            OpenAlternativeConfig {
                dir: Some(CommandLine::parse_for_file("code {path}", Os::Linux).unwrap()),
                extensions: BTreeMap::from([(
                    "pdf".to_owned(),
                    CommandLine::parse_for_file("sumatrapdf {path}", Os::Linux).unwrap()
                )]),
            }
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
        let error = parse("[filelist]\nnew_dir_goes_to_dir = \n", Os::Linux).unwrap_err();

        assert_eq!(error.line, 2);
    }

    #[test]
    fn unknown_items_are_reported_and_the_rest_is_read() {
        let text = "[filelist]\nfoo = 1\nnew_dir_goes_to_dir = false\n[colors.light]\nbar = 2\n[nothing]\nx = 1\n";

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert!(!config.filelist.new_dir_goes_to_dir);
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
            "[programs]\neditor = \"x 'a b'\"\n[commands.x]\ncommand_line = \"x 'a b'\"\n",
            Os::Windows,
        )
        .unwrap();

        assert_eq!(diagnostics, Vec::new());
        assert_eq!(
            config
                .programs
                .editor
                .expand(&Expansion::default())
                .unwrap(),
            ["'a", "b'"].map(OsString::from)
        );
        assert_eq!(
            config.commands["x"].expand(&Expansion::default()).unwrap(),
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
    fn limits_are_integers_of_one_or_more() {
        let read = |value: &str| {
            let text = format!(
                "[filelist]\nmax_history = {value}\nmax_find_matches = {value}\n\
                 [preview.text]\nmax_bytes = {value}\n[preview.image]\nmax_bytes = {value}\n\
                 [log]\nmax_lines = {value}\n"
            );
            let (config, diagnostics) = parse(&text, Os::Linux).unwrap();
            (
                config.filelist.max_history,
                config.filelist.max_find_matches,
                config.preview.text.max_bytes,
                config.preview.image.max_bytes,
                config.log.max_lines,
                paths(&diagnostics).len(),
            )
        };

        assert_eq!(read("1"), (1, 1, 1, 1, 1, 0));
        for invalid in ["0", "\"\""] {
            assert_eq!(
                read(invalid),
                (100, 10_000, 16 * 1024 * 1024, 64 * 1024 * 1024, 1000, 5),
                "{invalid}"
            );
        }
    }

    #[test]
    fn mark_compare_tolerance_is_an_integer_of_zero_or_more() {
        let read = |value: &str| {
            let text = format!("[filelist]\nmark_compare_tolerance_ms = {value}\n");
            let (config, diagnostics) = parse(&text, Os::Linux).unwrap();
            (
                config.filelist.mark_compare_tolerance_ms,
                paths(&diagnostics).len(),
            )
        };

        assert_eq!(read("0"), (0, 0));
        for invalid in ["-1", "\"\""] {
            assert_eq!(read(invalid), (2000, 1), "{invalid}");
        }
    }

    #[test]
    fn search_match_reads_anywhere_without_a_warning() {
        let read = |value: &str| {
            let text = format!("[filelist]\nsearch_match = {value}\n");
            let (config, diagnostics) = parse(&text, Os::Linux).unwrap();
            (config.filelist.search_match, paths(&diagnostics).len())
        };

        assert_eq!(read("\"anywhere\""), (SearchMatch::Anywhere, 0));
    }

    #[test]
    fn column_widths_are_integers_within_their_ranges() {
        let read = |text: &str| {
            let (config, diagnostics) = parse(text, Os::Linux).unwrap();
            let filelist = config.filelist;
            (
                (
                    filelist.size_digits,
                    filelist.extension_chars,
                    filelist.min_name_chars,
                ),
                diagnostics,
            )
        };

        assert_eq!(
            read("[filelist]\nsize_digits = 5\nextension_chars = 1\nmin_name_chars = 1\n"),
            ((5, 1, 1), Vec::new())
        );
        assert_eq!(
            read("[filelist]\nsize_digits = 20\nextension_chars = 255\nmin_name_chars = 255\n"),
            ((20, 255, 255), Vec::new())
        );
        let (widths, diagnostics) = read("[filelist]\nextension_chars = 0\nmin_name_chars = 256\n");
        assert_eq!(widths, (9, 4, 8));
        assert_eq!(diagnostics.len(), 2);
        for invalid in ["4", "21", "\"12\""] {
            let (widths, diagnostics) = read(&format!("[filelist]\nsize_digits = {invalid}\n"));
            assert_eq!(widths, (9, 4, 8), "{invalid}");
            assert_eq!(
                diagnostics[0].kind,
                DiagnosticKind::InvalidValue {
                    expected: "an integer from 5 to 20".to_owned()
                },
                "{invalid}"
            );
        }
    }

    #[test]
    fn invalid_values_fall_back_item_by_item() {
        let text = r##"
[filelist]
size_format = "bogus"
[preview.image]
placement_from_left = "full"
placement_from_right = true
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
extensions = 1
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
                "preview.image.placement_from_left",
                "preview.image.placement_from_right",
                "programs.editor",
                "font.log.size",
                "font.size",
                "colors.theme",
                "colors.light.extensions",
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
command_line = "wt"
[commands.broken]
command_line = "x 'unterminated"
[keys.filelist]
"F1" = "$term"
"F2" = "$missing"
"F3" = "$broken"
"F4" = "no_such_command"
"Shift+!" = "copy"
"F5" = 3
[keys.preview.image]
"F6" = "copy"
"Up" = "nop"
[keys.preview.other]
[keys.text]
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
                "commands.broken.command_line",
                "keys.filelist",
                "keys.filelist.F2",
                "keys.filelist.F3",
                "keys.filelist.F4",
                "keys.filelist.F5",
                "keys.preview.image.F6",
                "keys.preview.other",
                "keys.text",
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
    fn keys_that_no_key_press_gives_are_reported() {
        let (config, diagnostics) = parse(
            "[keys.filelist]\n\" \" = \"copy\"\n\"\\t\" = \"move\"\n\"Ctrl+ \" = \"move\"\n\"Space\" = \"delete\"\n",
            Os::Linux,
        )
        .unwrap();

        let invalid: Vec<_> = diagnostics
            .iter()
            .filter(|diagnostic| matches!(diagnostic.kind, DiagnosticKind::InvalidKey { .. }))
            .collect();
        assert_eq!(invalid.len(), 3, "{diagnostics:?}");
        assert_eq!(diagnostics.len(), 3, "{diagnostics:?}");
        let space: Key = "Space".parse().unwrap();
        assert_eq!(
            config.keys.lookup(Context::FileList, &space),
            Some(Command::Delete)
        );
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
[keys.preview.image]
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
    fn extension_colors_are_keyed_in_lowercase_and_bad_items_dropped_alone() {
        let text = r##"
[colors.light.extensions]
ZIP = "#a02020"
7z = "#123"
"" = "#000000"
"tar.gz" = "#000000"
png = "red"
gz = 1
[colors.dark.extensions]
Jpg = "#ffffff"
jpg = "#000000"
jpeg = "#eeeeee"
"##;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(
            config.colors.light.extensions,
            BTreeMap::from([
                ("7z".to_owned(), Color::new(0x11, 0x22, 0x33)),
                ("zip".to_owned(), Color::new(0xa0, 0x20, 0x20)),
            ])
        );
        assert_eq!(
            config.colors.dark.extensions,
            BTreeMap::from([("jpeg".to_owned(), Color::new(0xee, 0xee, 0xee))])
        );
        let diagnostic = |path: &str, kind| Diagnostic {
            path: path.to_owned(),
            kind,
        };
        let invalid_color = || DiagnosticKind::InvalidValue {
            expected: "a color like #rrggbb".to_owned(),
        };
        assert_eq!(
            diagnostics,
            [
                diagnostic(
                    "colors.light.extensions",
                    DiagnosticKind::InvalidKey { key: String::new() }
                ),
                diagnostic(
                    "colors.light.extensions",
                    DiagnosticKind::InvalidKey {
                        key: "tar.gz".to_owned()
                    }
                ),
                diagnostic("colors.light.extensions.gz", invalid_color()),
                diagnostic("colors.light.extensions.png", invalid_color()),
                diagnostic(
                    "colors.dark.extensions",
                    DiagnosticKind::DuplicateKey {
                        key: "jpg".to_owned(),
                        spellings: vec!["Jpg".to_owned(), "jpg".to_owned()],
                    }
                ),
            ]
        );
    }

    #[test]
    fn open_alternative_rules_are_keyed_in_lowercase_and_bad_items_dropped_alone() {
        let text = r#"
[open_alternative]
dir = "x 'unterminated"
other = 1
[open_alternative.extensions]
PDF = "view {path}"
"" = "view"
"tar.gz" = "view"
md = " "
txt = 1
Jpg = "a"
jpg = "b"
"#;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(
            config.open_alternative,
            OpenAlternativeConfig {
                dir: None,
                extensions: BTreeMap::from([(
                    "pdf".to_owned(),
                    CommandLine::parse_for_file("view {path}", Os::Linux).unwrap()
                )]),
            }
        );
        let diagnostic = |path: &str, kind| Diagnostic {
            path: path.to_owned(),
            kind,
        };
        assert_eq!(
            diagnostics,
            [
                diagnostic(
                    "open_alternative.dir",
                    DiagnosticKind::InvalidCommandLine(
                        CommandLineError::UnterminatedQuoteOrBackslash
                    )
                ),
                diagnostic(
                    "open_alternative.extensions",
                    DiagnosticKind::InvalidKey { key: String::new() }
                ),
                diagnostic(
                    "open_alternative.extensions",
                    DiagnosticKind::InvalidKey {
                        key: "tar.gz".to_owned()
                    }
                ),
                diagnostic(
                    "open_alternative.extensions",
                    DiagnosticKind::DuplicateKey {
                        key: "jpg".to_owned(),
                        spellings: vec!["Jpg".to_owned(), "jpg".to_owned()],
                    }
                ),
                diagnostic(
                    "open_alternative.extensions.md",
                    DiagnosticKind::InvalidCommandLine(CommandLineError::Empty)
                ),
                diagnostic(
                    "open_alternative.extensions.txt",
                    DiagnosticKind::InvalidValue {
                        expected: "a string".to_owned()
                    }
                ),
                diagnostic("open_alternative.other", DiagnosticKind::UnknownItem),
            ]
        );
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
    fn a_command_is_a_line_or_a_table_with_command_line() {
        let text = r#"
[commands]
broken = "x 'unterminated"
line = "wt -d {dir}"
number = 3
[commands.table]
command_line = "wt -d {dir}"
"#;

        let (config, diagnostics) = parse(text, Os::Linux).unwrap();

        assert_eq!(
            config
                .commands
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["line", "table"]
        );
        assert_eq!(config.commands["line"], config.commands["table"]);
        assert_eq!(paths(&diagnostics), ["commands.broken", "commands.number"]);
        assert!(matches!(
            diagnostics[0].kind,
            DiagnosticKind::InvalidCommandLine(_)
        ));
        assert_eq!(
            diagnostics[1].kind,
            DiagnosticKind::InvalidValue {
                expected: "a string or a table".to_owned()
            }
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
[commands.incomplete]
[commands.term]
command_line = "wt"
[keys.filelist]
"F1" = "$incomplete"
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
            ["bookmarks[0]", "commands.incomplete", "keys.filelist.F1"]
        );
        assert_eq!(
            diagnostics[0].kind,
            DiagnosticKind::MissingItem {
                item: "path".to_owned()
            }
        );
        assert_eq!(
            diagnostics[1].kind,
            DiagnosticKind::MissingItem {
                item: "command_line".to_owned()
            }
        );
        assert_eq!(
            diagnostics[0].to_string(),
            "Config: missing path in bookmarks[0]"
        );
    }

    #[test]
    fn merging_overlays_key_tables_by_normalized_key() {
        let mut base = table(
            "[keys.filelist]\n\"a\" = \"copy\"\n\"F1\" = \"copy\"\n[keys.preview.text]\n\"j\" = \"copy\"\n",
        );

        merge(
            &mut base,
            table("[keys.filelist]\n\"A\" = \"move\"\n[keys.preview.text]\n\"J\" = \"move\"\n"),
        );

        assert_eq!(
            base,
            table(
                "[keys.filelist]\n\"A\" = \"move\"\n\"F1\" = \"copy\"\n[keys.preview.text]\n\"J\" = \"move\"\n"
            )
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

    #[test]
    fn later_files_merge_over_earlier_ones() {
        let (config, diagnostics) = parse_layers(
            in_one_file(&[
                "[filelist]\nrequire_marks = false\nsize_format = \"comma\"\n",
                "extends = \"a.toml\"\n[filelist]\nsize_format = \"human\"\nnew_dir_goes_to_dir = false\n",
                "extends = [\"b.toml\"]\n[filelist]\nnew_dir_goes_to_dir = true\n",
            ]),
            Os::Linux,
        )
        .unwrap();

        assert_eq!(diagnostics, Vec::new());
        assert!(!config.filelist.require_marks);
        assert_eq!(config.filelist.size_format, SizeFormat::Human);
        assert!(config.filelist.new_dir_goes_to_dir);
    }

    #[test]
    fn bookmarks_are_taken_from_the_directory_of_the_last_file_that_has_them() {
        let (config, diagnostics) = parse_layers(
            [
                (
                    Path::new("/a/base.toml"),
                    "[[bookmarks]]\nname = \"a\"\npath = \"a\"\n",
                ),
                (
                    Path::new("/b/middle.toml"),
                    "[[bookmarks]]\nname = \"b\"\npath = \"b\"\n",
                ),
                (Path::new(FILE), "[filelist]\nrequire_marks = false\n"),
            ],
            Os::Linux,
        )
        .unwrap();

        assert_eq!(diagnostics, []);
        assert_eq!(
            config
                .bookmarks
                .iter()
                .map(|bookmark| (bookmark.name.as_str(), bookmark.dir.as_path()))
                .collect::<Vec<_>>(),
            [("b", Path::new("/b"))]
        );
    }

    #[test]
    fn an_empty_string_takes_back_what_a_file_below_set() {
        let (config, diagnostics) = parse_layers(
            in_one_file(&[
                r##"
[filelist]
size_format = "human"
[programs]
editor = "code {path}"
[font]
family = "Menlo"
[colors.light]
fg = "#111111"
[colors.light.extensions]
ZIP = "#222222"
png = "#333333"
[open_alternative]
dir = "code {path}"
[open_alternative.extensions]
PDF = "view {path}"
txt = "edit {path}"
[commands]
term = "wt"
[commands.build]
command_line = "make"
[commands.keep]
command_line = "keep"
[keys.filelist]
"c" = "move"
"##,
                r#"
[filelist]
size_format = ""
[programs]
editor = ""
[font]
family = ""
[colors.light]
fg = ""
[colors.light.extensions]
zip = ""
[open_alternative]
dir = ""
[open_alternative.extensions]
pdf = ""
[commands]
term = ""
[commands.build]
command_line = ""
[keys.filelist]
"C" = ""
"#,
            ]),
            Os::Linux,
        )
        .unwrap();

        let defaults = Config::default_for(Os::Linux);
        assert_eq!(diagnostics, []);
        assert_eq!(config.filelist.size_format, defaults.filelist.size_format);
        assert_eq!(config.programs.editor, defaults.programs.editor);
        assert_eq!(config.font.default.family, None);
        assert_eq!(config.colors.light.fg, defaults.colors.light.fg);
        assert_eq!(
            config.colors.light.extensions,
            BTreeMap::from([("png".to_owned(), Color::new(0x33, 0x33, 0x33))])
        );
        assert_eq!(config.open_alternative.dir, None);
        assert_eq!(
            config
                .open_alternative
                .extensions
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["txt"]
        );
        assert_eq!(
            config
                .commands
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["keep"]
        );
        let key: Key = "c".parse().unwrap();
        assert_eq!(
            config.keys.lookup(Context::FileList, &key),
            defaults.keys.lookup(Context::FileList, &key)
        );
        assert_ne!(config.keys.lookup(Context::FileList, &key), None);
    }

    #[test]
    fn an_empty_string_in_an_unknown_item_is_still_reported() {
        let (_, diagnostics) = parse("[filelist]\nbogus = \"\"\n", Os::Linux).unwrap();

        assert_eq!(paths(&diagnostics), ["filelist.bogus"]);
    }

    #[test]
    fn a_syntax_error_in_any_file_names_that_file() {
        let error = parse_layers(in_one_file(&["", "[filelist\n", ""]), Os::Linux).unwrap_err();

        assert_eq!(error.0, 1);
    }

    #[test]
    fn extends_names_one_file_or_a_list_of_them() {
        assert_eq!(extends("extends = \"a.toml\"\n"), ["a.toml"]);
        assert_eq!(
            extends("extends = [\"a.toml\", \"b.toml\"]\n"),
            ["a.toml", "b.toml"]
        );
        assert_eq!(extends("[filelist]\n"), Vec::<String>::new());
        assert_eq!(
            extends("extends = \"a.toml\"\n[filelist\n"),
            Vec::<String>::new()
        );
    }

    #[test]
    fn an_extends_of_the_wrong_type_is_reported_and_skipped() {
        let (_, diagnostics) = parse("extends = 1\n", Os::Linux).unwrap();
        assert_eq!(
            diagnostics[0].to_string(),
            "Config: invalid value for extends (expected a string or an array of strings)"
        );
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(extends("extends = 1\n"), Vec::<String>::new());

        let text = "extends = [\"a.toml\", 1, \"\"]\n";
        let (_, diagnostics) = parse(text, Os::Linux).unwrap();
        assert_eq!(
            diagnostics[0].to_string(),
            "Config: invalid value for extends[1] (expected a non-empty string)"
        );
        assert_eq!(paths(&diagnostics), ["extends[1]", "extends[2]"]);
        assert_eq!(extends(text), ["a.toml"]);
    }

    #[test]
    fn an_empty_extends_names_no_file() {
        let (_, diagnostics) = parse("extends = \"\"\n", Os::Linux).unwrap();
        assert_eq!(diagnostics, []);
        assert_eq!(extends("extends = \"\"\n"), Vec::<String>::new());
    }

    fn paths(diagnostics: &[Diagnostic]) -> Vec<&str> {
        diagnostics.iter().map(|d| d.path.as_str()).collect()
    }

    fn parse(text: &str, os: Os) -> Result<(Config, Vec<Diagnostic>), SyntaxError> {
        parse_layers([(Path::new(FILE), text)], os).map_err(|(_, error)| error)
    }

    fn table(text: &str) -> Table {
        toml::from_str(text).unwrap()
    }

    fn bookmark(name: &str, path: &str) -> Bookmark {
        Bookmark {
            name: name.to_owned(),
            path: PathBuf::from(path),
            dir: Path::new(FILE).parent().unwrap().to_path_buf(),
            key: None,
        }
    }

    fn in_one_file<'a>(texts: &[&'a str]) -> Vec<(&'a Path, &'a str)> {
        texts.iter().map(|text| (Path::new(FILE), *text)).collect()
    }
}
