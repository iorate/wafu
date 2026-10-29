use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use thiserror::Error;

use crate::app::Os;
use crate::external::{CommandLine, CommandLineError};
use crate::keymap::KeyMap;
use crate::listing::SortSpec;

mod decode;

/// The configuration file with every item commented out, written when there is none.
pub const CONFIG_TEMPLATE: &str = include_str!("../config.template.toml");

const FONT_SIZE_DEFAULT: u16 = 12;
const FONT_SIZE_MIN: u16 = 6;
const FONT_SIZE_MAX: u16 = 72;

/// The configuration after the OS section has been merged in and every
/// unusable item has been replaced by its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub general: General,
    pub appearance: Appearance,
    pub colors: ColorSet,
    pub initial: Initial,
    /// As written, before `~` is expanded.
    pub bookmarks: Vec<PathBuf>,
    pub commands: BTreeMap<String, CommandLine>,
    pub keys: KeyMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct General {
    pub mkdir_then_enter: bool,
    pub new_text_creates_file: bool,
    pub require_marks: bool,
    pub editor: CommandLine,
    pub hide_log_in_preview: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appearance {
    pub theme: Theme,
    pub size_format: SizeFormat,
    pub font: FontSet,
}

/// How a size in bytes is written in the file list and the information line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SizeFormat {
    #[default]
    Plain,
    Comma,
    Human,
}

impl SizeFormat {
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "plain" => Some(Self::Plain),
            "comma" => Some(Self::Comma),
            "human" => Some(Self::Human),
            _ => None,
        }
    }

    /// A sample as wide as the widest size the file list should fit without
    /// clipping, for sizing the column.
    #[must_use]
    pub fn column_sample(self) -> &'static str {
        match self {
            Self::Plain => "000000000000",
            Self::Comma => "000,000,000,000",
            Self::Human => "0000.0K",
        }
    }

    /// Writes `bytes` in this format. `Human` follows `ls -h`: powers of
    /// 1024, rounded up, one decimal below 10.
    #[must_use]
    pub fn format(self, bytes: u64) -> String {
        match self {
            Self::Plain => bytes.to_string(),
            Self::Comma => {
                let digits = bytes.to_string();
                let mut out = String::with_capacity(digits.len() + digits.len() / 3);
                for (i, c) in digits.chars().enumerate() {
                    if i > 0 && (digits.len() - i).is_multiple_of(3) {
                        out.push(',');
                    }
                    out.push(c);
                }
                out
            }
            Self::Human => human_readable(bytes),
        }
    }
}

const HUMAN_UNITS: [&str; 6] = ["K", "M", "G", "T", "P", "E"];

fn human_readable(bytes: u64) -> String {
    if bytes < 1024 {
        return bytes.to_string();
    }
    let bytes = u128::from(bytes);
    let mut unit = 0;
    let mut scale: u128 = 1024;
    while unit + 1 < HUMAN_UNITS.len() && bytes >= scale * 1024 {
        scale *= 1024;
        unit += 1;
    }
    let tenths = (bytes * 10).div_ceil(scale);
    if tenths < 100 {
        return format!("{}.{}{}", tenths / 10, tenths % 10, HUMAN_UNITS[unit]);
    }
    let whole = bytes.div_ceil(scale);
    // Rounding up can reach the next unit, as 1048575 bytes does.
    if whole >= 1024 && unit + 1 < HUMAN_UNITS.len() {
        return format!("1.0{}", HUMAN_UNITS[unit + 1]);
    }
    format!("{whole}{}", HUMAN_UNITS[unit])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Auto,
    Light,
    Dark,
}

/// The default font and the fonts of the areas that may differ from it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FontSet {
    pub default: Font,
    pub path: Font,
    pub filelist: Font,
    pub info: Font,
    pub log: Font,
    pub status: Font,
    pub preview: Font,
    pub popup: Font,
}

/// A font as configured; an item that is `None` falls back to the default font.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Font {
    pub family: Option<String>,
    /// In pixels.
    pub size: Option<u16>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorSet {
    pub light: Colors,
    pub dark: Colors,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Colors {
    pub fg: Color,
    pub bg: Color,
    pub mark: Color,
    pub cursor: Color,
    pub dir: Color,
    pub link: Color,
    pub exec: Color,
    pub broken: Color,
    pub fifo: Color,
    pub socket: Color,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// The display state of a new pane when there is no state file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Initial {
    pub sort: SortSpec,
    pub show_hidden: bool,
}

/// A warning about one item of the configuration, shown in the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The item, such as `appearance.theme`.
    pub path: String,
    pub kind: DiagnosticKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticKind {
    UnknownItem,
    InvalidValue { expected: String },
    MissingItem { item: String },
    InvalidKey { key: String },
    UnknownCommand { command: String },
    Unavailable { command: String },
    InvalidCommandLine(CommandLineError),
}

/// The file could not be read as TOML at all.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{line}: {message}")]
pub struct SyntaxError {
    pub line: usize,
    pub message: String,
}

/// Reads a configuration file leniently: a syntax error rejects the whole
/// file, while an unknown or unusable item is reported and ignored.
pub fn parse_config(text: &str, os: Os) -> Result<(Config, Vec<Diagnostic>), SyntaxError> {
    decode::parse(text, os)
}

impl Config {
    #[must_use]
    pub fn default_for(os: Os) -> Self {
        Self {
            general: General {
                mkdir_then_enter: true,
                new_text_creates_file: true,
                require_marks: true,
                editor: default_editor(os),
                hide_log_in_preview: false,
            },
            appearance: Appearance {
                theme: Theme::Auto,
                size_format: SizeFormat::Plain,
                font: FontSet {
                    default: Font {
                        family: None,
                        size: Some(FONT_SIZE_DEFAULT),
                    },
                    ..FontSet::default()
                },
            },
            colors: ColorSet {
                light: Colors {
                    fg: Color::new(0x1e, 0x1e, 0x1e),
                    bg: Color::new(0xff, 0xff, 0xff),
                    mark: Color::new(0xdc, 0xdc, 0xdc),
                    cursor: Color::new(0x1e, 0x1e, 0x1e),
                    dir: Color::new(0x1f, 0x5f, 0xbf),
                    link: Color::new(0x0e, 0x7c, 0x86),
                    exec: Color::new(0x2a, 0x8c, 0x2a),
                    broken: Color::new(0xc0, 0x39, 0x2b),
                    fifo: Color::new(0xb8, 0x86, 0x0b),
                    socket: Color::new(0xb0, 0x3a, 0x9a),
                },
                dark: Colors {
                    fg: Color::new(0xd4, 0xd4, 0xd4),
                    bg: Color::new(0x1e, 0x1e, 0x1e),
                    mark: Color::new(0x3c, 0x3c, 0x3c),
                    cursor: Color::new(0xd4, 0xd4, 0xd4),
                    dir: Color::new(0x6f, 0xa8, 0xff),
                    link: Color::new(0x4f, 0xd1, 0xe0),
                    exec: Color::new(0x5f, 0xd7, 0x5f),
                    broken: Color::new(0xff, 0x6b, 0x6b),
                    fifo: Color::new(0xf0, 0xc0, 0x50),
                    socket: Color::new(0xe5, 0x8a, 0xe0),
                },
            },
            initial: Initial {
                sort: SortSpec::default(),
                show_hidden: false,
            },
            bookmarks: Vec::new(),
            commands: BTreeMap::new(),
            keys: KeyMap::default_for(os),
        }
    }
}

impl Font {
    /// The font to use for an area: its own items, else the default's, else
    /// `family` and the built-in size.
    #[must_use]
    pub fn resolve(&self, default: &Self, family: &str) -> (String, u16) {
        (
            self.family
                .clone()
                .or_else(|| default.family.clone())
                .unwrap_or_else(|| family.to_owned()),
            self.size.or(default.size).unwrap_or(FONT_SIZE_DEFAULT),
        )
    }
}

impl Theme {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "auto" => Self::Auto,
            "light" => Self::Light,
            "dark" => Self::Dark,
            _ => return None,
        })
    }
}

impl Color {
    #[must_use]
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Parses `#rrggbb` or `#rgb`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let hex = text.strip_prefix('#')?;
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return None;
        }
        let digits: Vec<u8> = hex
            .chars()
            .filter_map(|c| c.to_digit(16))
            .map(|d| u8::try_from(d).unwrap_or(0))
            .collect();
        let [r, g, b] = match digits.as_slice() {
            [r1, r2, g1, g2, b1, b2] => [r1 * 16 + r2, g1 * 16 + g2, b1 * 16 + b2],
            [r, g, b] => [r * 17, g * 17, b * 17],
            _ => return None,
        };
        Some(Self::new(r, g, b))
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = &self.path;
        match &self.kind {
            DiagnosticKind::UnknownItem => write!(f, "config: unknown item {path}"),
            DiagnosticKind::InvalidValue { expected } => {
                write!(f, "config: invalid value for {path} (expected {expected})")
            }
            DiagnosticKind::MissingItem { item } => {
                write!(f, "config: missing {item} in {path}")
            }
            DiagnosticKind::InvalidKey { key } => {
                write!(f, "config: invalid key {key:?} in {path}")
            }
            DiagnosticKind::UnknownCommand { command } => {
                write!(f, "config: unknown command {command:?} for {path}")
            }
            DiagnosticKind::Unavailable { command } => {
                write!(f, "config: {command} is not available in {path}")
            }
            DiagnosticKind::InvalidCommandLine(error) => {
                write!(f, "config: invalid command line for {path}: {error}")
            }
        }
    }
}

pub(crate) fn monospace_family(os: Os) -> &'static str {
    match os {
        Os::Windows => "Consolas",
        Os::MacOs => "Menlo",
        Os::Linux => "DejaVu Sans Mono",
    }
}

fn default_editor(os: Os) -> CommandLine {
    let text = match os {
        Os::Windows => "notepad {paths}",
        Os::MacOs => "open -t {paths}",
        Os::Linux => "xdg-open {paths}",
    };
    CommandLine::parse(text).expect("the default editor should parse")
}

#[cfg(test)]
mod tests {
    #[test]
    fn size_formats_follow_ls() {
        use super::SizeFormat::{Comma, Human, Plain};

        assert_eq!(Plain.format(1_234_567), "1234567");
        assert_eq!(Comma.format(0), "0");
        assert_eq!(Comma.format(999), "999");
        assert_eq!(Comma.format(1000), "1,000");
        assert_eq!(Comma.format(1_234_567), "1,234,567");
        assert_eq!(Human.format(0), "0");
        assert_eq!(Human.format(1023), "1023");
        assert_eq!(Human.format(1024), "1.0K");
        assert_eq!(Human.format(1025), "1.1K");
        assert_eq!(Human.format(1536), "1.5K");
        assert_eq!(Human.format(10239), "10K");
        assert_eq!(Human.format(10240), "10K");
        assert_eq!(Human.format(1_048_575), "1.0M");
        assert_eq!(Human.format(1_048_576), "1.0M");
        assert_eq!(Human.format(345 * 1024 * 1024), "345M");
        assert_eq!(Human.format(u64::MAX), "16E");
    }

    use super::*;

    #[test]
    fn color_parses_long_and_short_forms() {
        assert_eq!(Color::parse("#1e1e1e"), Some(Color::new(0x1e, 0x1e, 0x1e)));
        assert_eq!(Color::parse("#ABC"), Some(Color::new(0xaa, 0xbb, 0xcc)));
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color::parse("1e1e1e"), None);
        assert_eq!(Color::parse("#1e1e1g"), None);
    }

    #[test]
    fn font_resolves_through_the_default() {
        let default = Font {
            family: Some("A".to_owned()),
            size: Some(14),
        };

        assert_eq!(Font::default().resolve(&default, "M"), ("A".to_owned(), 14));
        assert_eq!(
            Font {
                family: Some("B".to_owned()),
                size: None,
            }
            .resolve(&default, "M"),
            ("B".to_owned(), 14)
        );
        assert_eq!(
            Font::default().resolve(&Font::default(), "M"),
            ("M".to_owned(), FONT_SIZE_DEFAULT)
        );
    }

    #[test]
    fn diagnostics_read_as_log_lines() {
        let diagnostic = Diagnostic {
            path: "keys.filelist.F2".to_owned(),
            kind: DiagnosticKind::UnknownCommand {
                command: "$open".to_owned(),
            },
        };

        assert_eq!(
            diagnostic.to_string(),
            "config: unknown command \"$open\" for keys.filelist.F2"
        );
    }

    #[test]
    fn template_with_its_items_uncommented_is_the_default() {
        let text: String = CONFIG_TEMPLATE
            .lines()
            .map(|line| match line.strip_prefix("# ") {
                Some(item)
                    if item
                        .split_once(" = ")
                        .is_some_and(|(key, _)| !key.is_empty() && !key.contains(' ')) =>
                {
                    item
                }
                _ => line,
            })
            .fold(String::new(), |mut text, line| {
                text.push_str(line);
                text.push('\n');
                text
            });

        for os in [Os::Windows, Os::MacOs, Os::Linux] {
            let (config, diagnostics) = parse_config(&text, os).unwrap();
            assert_eq!(diagnostics, Vec::new(), "{os:?}");
            assert_eq!(config, Config::default_for(os), "{os:?}");
        }
    }

    #[test]
    fn template_as_written_is_all_comments() {
        let (config, diagnostics) = parse_config(CONFIG_TEMPLATE, Os::Linux).unwrap();

        assert_eq!(diagnostics, Vec::new());
        assert_eq!(config, Config::default_for(Os::Linux));
    }
}
