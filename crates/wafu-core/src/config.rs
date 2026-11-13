use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use thiserror::Error;

use crate::app::Os;
use crate::external::{CommandLine, CommandLineError};
use crate::keymap::KeyMap;

mod decode;

/// The configuration file written when there is none.
// Only a link to the documentation: the file is never rewritten once created,
// so any description or default value in it would go stale or pin an old
// default after an upgrade.
pub const CONFIG_TEMPLATE: &str = include_str!("../config.template.toml");

const FONT_SIZE_DEFAULT: u16 = 12;
const FONT_SIZE_MIN: u16 = 6;
const FONT_SIZE_MAX: u16 = 72;
const HUMAN_UNITS: [&str; 6] = ["K", "M", "G", "T", "P", "E"];

/// The configuration after the OS section has been merged in and every
/// unusable item has been replaced by its default.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub general: General,
    pub appearance: Appearance,
    pub colors: ColorSet,
    pub bookmarks: Vec<Bookmark>,
    pub commands: BTreeMap<String, CommandLine>,
    pub keys: KeyMap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    /// What the bookmarks popup shows.
    pub name: String,
    /// As written, before `~` is expanded.
    pub path: PathBuf,
    /// The key that picks the bookmark at once: a digit or a lowercase letter.
    pub key: Option<char>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct General {
    pub mkdir_then_enter: bool,
    pub new_text_creates_file: bool,
    pub extract_creates_dir: bool,
    pub require_marks: bool,
    pub editor: CommandLine,
    pub hide_log_in_preview: bool,
    pub image_preview_from_left: ImagePlacement,
    pub image_preview_from_right: ImagePlacement,
    pub use_login_shell_env: bool,
    pub search_matches_anywhere: bool,
}

/// Where an image opened from a pane is previewed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImagePlacement {
    /// In place of the pane it was not opened from.
    OtherPane,
    /// In place of both panes.
    Full,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Appearance {
    pub theme: Theme,
    pub size_format: SizeFormat,
    /// How opaque the names of hidden entries are, in percent.
    pub hidden_opacity: u8,
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
    pub line_number: Color,
    pub mark: Color,
    pub cursor: Color,
    pub cursor_inactive: Color,
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

/// A warning about one item of the configuration, shown in the log. It has
/// no line number: the decoded table carries no positions, unlike a syntax
/// error.
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
    DuplicateKey { key: String, spellings: Vec<String> },
    DuplicateBookmarkKey { key: char, bookmarks: Vec<String> },
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
                // On by default because the macOS `open -t` does nothing for a
                // file that does not exist.
                new_text_creates_file: true,
                extract_creates_dir: true,
                require_marks: true,
                editor: default_editor(os),
                hide_log_in_preview: false,
                image_preview_from_left: ImagePlacement::OtherPane,
                image_preview_from_right: ImagePlacement::Full,
                use_login_shell_env: true,
                search_matches_anywhere: true,
            },
            appearance: Appearance {
                theme: Theme::Auto,
                size_format: SizeFormat::Plain,
                // Hidden entries keep their kind color, which has less contrast
                // against the background than `fg`; any lower and they get hard
                // to read.
                hidden_opacity: 70,
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
                    line_number: Color::new(0x62, 0x62, 0x62),
                    mark: Color::new(0xdc, 0xdc, 0xdc),
                    cursor: Color::new(0x1e, 0x1e, 0x1e),
                    cursor_inactive: Color::new(0x7d, 0x7d, 0x7d),
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
                    line_number: Color::new(0x9d, 0x9d, 0x9d),
                    mark: Color::new(0x3c, 0x3c, 0x3c),
                    cursor: Color::new(0xd4, 0xd4, 0xd4),
                    cursor_inactive: Color::new(0x88, 0x88, 0x88),
                    dir: Color::new(0x6f, 0xa8, 0xff),
                    link: Color::new(0x4f, 0xd1, 0xe0),
                    exec: Color::new(0x5f, 0xd7, 0x5f),
                    broken: Color::new(0xff, 0x6b, 0x6b),
                    fifo: Color::new(0xf0, 0xc0, 0x50),
                    socket: Color::new(0xe5, 0x8a, 0xe0),
                },
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

impl ImagePlacement {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "other_pane" => Self::OtherPane,
            "full" => Self::Full,
            _ => return None,
        })
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
        let channel = |digits: &str| u8::from_str_radix(digits, 16).ok();
        let [r, g, b] = match hex.len() {
            6 => [
                channel(&hex[0..2])?,
                channel(&hex[2..4])?,
                channel(&hex[4..6])?,
            ],
            3 => [
                channel(&hex[0..1])? * 17,
                channel(&hex[1..2])? * 17,
                channel(&hex[2..3])? * 17,
            ],
            _ => return None,
        };
        Some(Self::new(r, g, b))
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = &self.path;
        match &self.kind {
            DiagnosticKind::UnknownItem => write!(f, "Config: unknown item {path}"),
            DiagnosticKind::InvalidValue { expected } => {
                write!(f, "Config: invalid value for {path} (expected {expected})")
            }
            DiagnosticKind::MissingItem { item } => {
                write!(f, "Config: missing {item} in {path}")
            }
            DiagnosticKind::InvalidKey { key } => {
                write!(f, "Config: invalid key \"{key}\" in {path}")
            }
            DiagnosticKind::DuplicateKey { key, spellings } => {
                write!(
                    f,
                    "Config: key {key} is written more than once in {path} ({}); all ignored",
                    spellings
                        .iter()
                        .map(|spelling| format!("\"{spelling}\""))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            DiagnosticKind::DuplicateBookmarkKey { key, bookmarks } => {
                write!(
                    f,
                    "Config: key {} is used by more than one bookmark ({}); all ignored",
                    key.to_ascii_uppercase(),
                    bookmarks.join(", ")
                )
            }
            DiagnosticKind::UnknownCommand { command } => {
                write!(f, "Config: unknown command \"{command}\" in {path}")
            }
            DiagnosticKind::Unavailable { command } => {
                write!(f, "Config: {command} is not available in {path}")
            }
            DiagnosticKind::InvalidCommandLine(error) => {
                write!(f, "Config: invalid command line for {path}: {error}")
            }
        }
    }
}

// Slint resolves only real family names; a generic one like `monospace` does
// nothing on any OS, so each OS gets a named default.
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
    CommandLine::parse(text, os).expect("the default editor should parse")
}

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

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn font_resolves_the_area_then_the_default_then_the_built_in() {
        let default = font(Some("A"), Some(16));

        assert_eq!(
            font(Some("B"), Some(20)).resolve(&default, "M"),
            ("B".to_owned(), 20)
        );
        assert_eq!(
            font(None, None).resolve(&default, "M"),
            ("A".to_owned(), 16)
        );
        assert_eq!(
            font(None, None).resolve(&font(None, None), "M"),
            ("M".to_owned(), FONT_SIZE_DEFAULT)
        );
    }

    #[test]
    fn font_family_and_size_fall_back_independently() {
        assert_eq!(
            font(None, Some(20)).resolve(&font(Some("A"), None), "M"),
            ("A".to_owned(), 20)
        );
    }

    #[test]
    fn color_parses_long_and_short_forms() {
        assert_eq!(Color::parse("#1e1e1e"), Some(Color::new(0x1e, 0x1e, 0x1e)));
        assert_eq!(Color::parse("#ABC"), Some(Color::new(0xaa, 0xbb, 0xcc)));
        assert_eq!(Color::parse("#"), None);
        assert_eq!(Color::parse("#12345"), None);
        assert_eq!(Color::parse("1e1e1e"), None);
        assert_eq!(Color::parse("#1e1e1g"), None);
        assert_eq!(Color::parse("#+1+1+1"), None);
        assert_eq!(Color::parse("#é1"), None);
    }

    #[test]
    fn diagnostics_quote_what_was_typed() {
        let line = |kind| {
            Diagnostic {
                path: "keys.filelist".to_owned(),
                kind,
            }
            .to_string()
        };

        assert_eq!(
            line(DiagnosticKind::InvalidKey {
                key: "Ctrl+\\".to_owned()
            }),
            r#"Config: invalid key "Ctrl+\" in keys.filelist"#
        );
        assert_eq!(
            line(DiagnosticKind::DuplicateKey {
                key: "A".to_owned(),
                spellings: vec!["A".to_owned(), "a".to_owned()],
            }),
            r#"Config: key A is written more than once in keys.filelist ("A", "a"); all ignored"#
        );
        assert_eq!(
            line(DiagnosticKind::DuplicateBookmarkKey {
                key: 'd',
                bookmarks: vec!["bookmarks[0]".to_owned(), "bookmarks[3]".to_owned()],
            }),
            "Config: key D is used by more than one bookmark (bookmarks[0], bookmarks[3]); all ignored"
        );
        assert_eq!(
            line(DiagnosticKind::UnknownCommand {
                command: "bogus".to_owned()
            }),
            r#"Config: unknown command "bogus" in keys.filelist"#
        );
    }

    #[test]
    fn template_as_written_is_the_default() {
        for os in [Os::Windows, Os::MacOs, Os::Linux] {
            let (config, diagnostics) = parse_config(CONFIG_TEMPLATE, os).unwrap();
            assert_eq!(diagnostics, Vec::new(), "{os:?}");
            assert_eq!(config, Config::default_for(os), "{os:?}");
        }
    }

    fn font(family: Option<&str>, size: Option<u16>) -> Font {
        Font {
            family: family.map(str::to_owned),
            size,
        }
    }
}
