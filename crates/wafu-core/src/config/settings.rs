use std::marker::PhantomData;

use toml::Value;

use super::{
    COLUMN_CHARS_MAX, Color, Colors, ColorsConfig, DiagnosticKind, FONT_SIZE_MAX, FONT_SIZE_MIN,
    FileListConfig, Font, ImagePlacement, ImagePreviewConfig, LogConfig, ProgramsConfig,
    SIZE_DIGITS_MAX, SIZE_DIGITS_MIN, SearchMatch, SizeFormat, TextPreviewConfig, Theme,
};
use crate::app::Os;
use crate::external::CommandLine;

// A row is written with a macro so that the field is named once: the kind
// fixes the type of the value, and the closures that store and load it are
// checked against the field's type.
macro_rules! setting {
    ($key:literal, $kind:expr, $first:ident $(. $rest:ident)*, $description:literal) => {
        Setting {
            key: $key,
            description: $description,
            empty_unsets: empty_unsets(&$kind),
            expected: || Kind::expected(&$kind),
            read: |target, value, os| {
                target.$first$(.$rest)* = Kind::parse(&$kind, value, os)?;
                Ok(())
            },
            write: |target| Kind::write(&$kind, &target.$first$(.$rest)*),
        }
    };
}

pub(super) const FILELIST: &[Setting<FileListConfig>] = &[
    setting!(
        "new_dir_goes_to_dir",
        Bool,
        new_dir_goes_to_dir,
        "Go to the directory that `new_dir` created."
    ),
    setting!(
        "new_text_creates_file",
        Bool,
        new_text_creates_file,
        "`new_text` creates the file before opening it in the editor."
    ),
    setting!(
        "extract_creates_dir",
        Bool,
        extract_creates_dir,
        "`extract` and `extract_here` put the contents in a directory named after the archive."
    ),
    setting!(
        "require_marks",
        Bool,
        require_marks,
        "`copy`, `move`, `delete`, and `trash` act only on marked items, never on the cursor item alone."
    ),
    setting!(
        "search_match",
        Choice::<SearchMatch>::NEW,
        search_match,
        "Where in a name `search` finds the text: `\"anywhere\"` or `\"start\"` (only at the start of a name)."
    ),
    setting!(
        "size_format",
        Choice::<SizeFormat>::NEW,
        size_format,
        "How the file list's size column writes sizes: `\"plain\"` (`1234567`), `\"comma\"` (`1,234,567`), or `\"human\"` (`1.2M`)."
    ),
    setting!(
        "size_digits",
        Count {
            min: SIZE_DIGITS_MIN,
            max: SIZE_DIGITS_MAX
        },
        size_digits,
        "How many digits the size column holds with `\"plain\"` and `\"comma\"`. A size with more digits is written as with `\"human\"`. With `\"human\"`, the column always holds five characters."
    ),
    setting!(
        "time_shows_full_year",
        Bool,
        time_format.full_year,
        "The file list's time column writes the year in full (`2026/01/02`). When `false`, only its last two digits (`26/01/02`)."
    ),
    setting!(
        "time_shows_seconds",
        Bool,
        time_format.seconds,
        "The file list's time column writes the seconds (`03:04:05`). When `false`, only the hours and minutes (`03:04`)."
    ),
    setting!(
        "extension_chars",
        COLUMN_CHARS,
        extension_chars,
        "How many characters of an extension the extension column holds after the dot. A longer extension ends in an ellipsis."
    ),
    setting!(
        "min_name_chars",
        COLUMN_CHARS,
        min_name_chars,
        "How many characters of a name a narrow pane still shows before cutting off the other columns."
    ),
    setting!(
        "max_history",
        Limit,
        max_history,
        "How many places each pane's history keeps, for `history`, `go_back`, and `go_forward`."
    ),
    setting!(
        "max_find_matches",
        Limit,
        max_find_matches,
        "How many matches `find` collects before it stops."
    ),
    setting!(
        "mark_compare_tolerance_ms",
        NonNegative,
        mark_compare_tolerance_ms,
        "How many milliseconds apart two modification times may be for `mark_compare` to take them as the same. `0` compares them exactly."
    ),
];

pub(super) const TEXT_PREVIEW: &[Setting<TextPreviewConfig>] = &[
    setting!(
        "hide_log",
        Bool,
        hide_log,
        "Hide the log while a text preview is open."
    ),
    setting!(
        "max_bytes",
        Limit,
        max_bytes,
        "How many bytes of a file the text preview reads. A longer file is cut there."
    ),
];

pub(super) const IMAGE_PREVIEW: &[Setting<ImagePreviewConfig>] = &[
    setting!(
        "placement_from_left",
        Choice::<ImagePlacement>::NEW,
        placement_from_left,
        "Where an image opened from the left pane is shown: `\"other_pane\"` (in place of the right pane, while the left pane stays) or `\"both_panes\"`."
    ),
    setting!(
        "placement_from_right",
        Choice::<ImagePlacement>::NEW,
        placement_from_right,
        "Where an image opened from the right pane is shown: `\"other_pane\"` (in place of the left pane, while the right pane stays) or `\"both_panes\"`."
    ),
    setting!(
        "hide_log",
        Bool,
        hide_log,
        "Hide the log while an image takes the place of both panes."
    ),
    setting!(
        "max_bytes",
        Limit,
        max_bytes,
        "The largest image file, in bytes, the image preview opens."
    ),
];

pub(super) const LOG: &[Setting<LogConfig>] = &[setting!(
    "max_lines",
    Limit,
    max_lines,
    "How many lines the log keeps."
)];

pub(super) const PROGRAMS: &[Setting<ProgramsConfig>] = &[
    setting!(
        "editor",
        Line,
        editor,
        "The command line that opens a file for editing, used by `edit`, `new_text`, `preview_edit`, and `edit_config`. `{paths}` is the file. It follows the rules of [external commands](#commands)."
    ),
    setting!(
        "use_login_shell_env",
        Bool,
        use_login_shell_env,
        "macOS and Linux: give the programs wafu starts the environment of your login shell."
    ),
];

pub(super) const FONT: &[Setting<Font>] = &[
    setting!(
        "family",
        Optional(Text),
        family,
        "A font family name. Empty means the monospaced font of the OS (Consolas on Windows, Menlo on macOS, DejaVu Sans Mono on Linux)."
    ),
    setting!(
        "size",
        Optional(FontSize),
        size,
        "The font size, in pixels."
    ),
];

pub(super) const COLORS_CONFIG: &[Setting<ColorsConfig>] = &[
    setting!(
        "theme",
        Choice::<Theme>::NEW,
        theme,
        "Which colors are used: `\"auto\"` (follow the OS), `\"light\"`, or `\"dark\"`."
    ),
    setting!(
        "hidden_opacity",
        Opacity,
        hidden_opacity,
        "How opaque the names of hidden items are: `0` is invisible, and `1` is like other items."
    ),
];

pub(super) const COLORS: &[Setting<Colors>] = &[
    setting!(
        "fg",
        Rgb,
        fg,
        "Text: regular files and the rest of the window's text"
    ),
    setting!("bg", Rgb, bg, "Background"),
    setting!(
        "line_number",
        Rgb,
        line_number,
        "Line numbers in the text preview, and offsets in its binary mode"
    ),
    setting!(
        "mark",
        Rgb,
        mark,
        "Background of marked rows and of the preview of a marked image"
    ),
    setting!("cursor", Rgb, cursor, "The cursor line"),
    setting!(
        "cursor_inactive",
        Rgb,
        cursor_inactive,
        "The cursor line while the window is not focused"
    ),
    setting!("dir", Rgb, dir, "Directories"),
    setting!("link", Rgb, link, "Symbolic links whose target exists"),
    setting!(
        "link_broken",
        Rgb,
        link_broken,
        "Symbolic links whose target does not exist"
    ),
    setting!("exec", Rgb, exec, "Executable files"),
    setting!("special", Rgb, special, "Named pipes and devices"),
    setting!("socket", Rgb, socket, "Sockets"),
];

const COLUMN_CHARS: Count = Count {
    min: 1,
    max: COLUMN_CHARS_MAX,
};

pub(super) struct Setting<T> {
    pub(super) key: &'static str,
    pub(super) description: &'static str,
    pub(super) empty_unsets: bool,
    pub(super) expected: fn() -> String,
    pub(super) read: fn(&mut T, Value, Os) -> Result<(), DiagnosticKind>,
    pub(super) write: fn(&T) -> Option<Value>,
}

pub(super) trait Kind {
    type Value;

    // Only for the kinds written as strings: an empty string is no value of
    // theirs, while the others have no such spare value.
    const EMPTY_UNSETS: bool = false;

    fn expected(&self) -> String;

    fn parse(&self, value: Value, os: Os) -> Result<Self::Value, DiagnosticKind>;

    fn write(&self, value: &Self::Value) -> Option<Value>;

    fn invalid(&self) -> DiagnosticKind {
        DiagnosticKind::InvalidValue {
            expected: self.expected(),
        }
    }
}

pub(super) trait Named: Copy + PartialEq + 'static {
    const NAMES: &'static [(&'static str, Self)];
}

pub(super) struct Bool;

pub(super) struct Choice<T>(PhantomData<T>);

pub(super) struct Count {
    min: usize,
    max: usize,
}

pub(super) struct Limit;

pub(super) struct NonNegative;

pub(super) struct FontSize;

pub(super) struct Opacity;

pub(super) struct Rgb;

pub(super) struct Text;

pub(super) struct Line;

pub(super) struct Optional<K>(K);

impl Kind for Bool {
    type Value = bool;

    fn expected(&self) -> String {
        "true or false".to_owned()
    }

    fn parse(&self, value: Value, _: Os) -> Result<bool, DiagnosticKind> {
        value.as_bool().ok_or_else(|| self.invalid())
    }

    fn write(&self, value: &bool) -> Option<Value> {
        Some(Value::Boolean(*value))
    }
}

impl<T> Choice<T> {
    pub(super) const NEW: Self = Self(PhantomData);
}

impl<T: Named> Kind for Choice<T> {
    type Value = T;

    const EMPTY_UNSETS: bool = true;

    fn expected(&self) -> String {
        let names: Vec<&str> = T::NAMES.iter().map(|(name, _)| *name).collect();
        if let [first, second] = names.as_slice() {
            return format!("{first} or {second}");
        }
        let mut text = names.join(", ");
        if let Some(last) = text.rfind(", ") {
            text.insert_str(last + 2, "or ");
        }
        text
    }

    fn parse(&self, value: Value, _: Os) -> Result<T, DiagnosticKind> {
        value
            .as_str()
            .and_then(|text| T::NAMES.iter().find(|(name, _)| *name == text))
            .map(|(_, choice)| *choice)
            .ok_or_else(|| self.invalid())
    }

    fn write(&self, value: &T) -> Option<Value> {
        T::NAMES
            .iter()
            .find(|(_, choice)| choice == value)
            .map(|(name, _)| Value::String((*name).to_owned()))
    }
}

impl Kind for Count {
    type Value = usize;

    fn expected(&self) -> String {
        format!("an integer from {} to {}", self.min, self.max)
    }

    fn parse(&self, value: Value, _: Os) -> Result<usize, DiagnosticKind> {
        value
            .as_integer()
            .and_then(|value| usize::try_from(value).ok())
            .filter(|count| (self.min..=self.max).contains(count))
            .ok_or_else(|| self.invalid())
    }

    fn write(&self, value: &usize) -> Option<Value> {
        i64::try_from(*value).ok().map(Value::Integer)
    }
}

impl Kind for Limit {
    type Value = usize;

    fn expected(&self) -> String {
        "an integer of 1 or more".to_owned()
    }

    fn parse(&self, value: Value, _: Os) -> Result<usize, DiagnosticKind> {
        value
            .as_integer()
            .filter(|value| *value >= 1)
            .and_then(|value| usize::try_from(value).ok())
            .ok_or_else(|| self.invalid())
    }

    fn write(&self, value: &usize) -> Option<Value> {
        i64::try_from(*value).ok().map(Value::Integer)
    }
}

impl Kind for NonNegative {
    type Value = u64;

    fn expected(&self) -> String {
        "an integer of 0 or more".to_owned()
    }

    fn parse(&self, value: Value, _: Os) -> Result<u64, DiagnosticKind> {
        value
            .as_integer()
            .and_then(|value| u64::try_from(value).ok())
            .ok_or_else(|| self.invalid())
    }

    fn write(&self, value: &u64) -> Option<Value> {
        i64::try_from(*value).ok().map(Value::Integer)
    }
}

impl Kind for FontSize {
    type Value = u16;

    fn expected(&self) -> String {
        format!("an integer from {FONT_SIZE_MIN} to {FONT_SIZE_MAX}")
    }

    fn parse(&self, value: Value, _: Os) -> Result<u16, DiagnosticKind> {
        value
            .as_integer()
            .and_then(|value| u16::try_from(value).ok())
            .filter(|size| (FONT_SIZE_MIN..=FONT_SIZE_MAX).contains(size))
            .ok_or_else(|| self.invalid())
    }

    fn write(&self, value: &u16) -> Option<Value> {
        Some(Value::Integer(i64::from(*value)))
    }
}

impl Kind for Opacity {
    type Value = u8;

    fn expected(&self) -> String {
        "a number from 0 to 1".to_owned()
    }

    fn parse(&self, value: Value, _: Os) -> Result<u8, DiagnosticKind> {
        let fraction = match value {
            Value::Float(value) => value,
            Value::Integer(0) => 0.0,
            Value::Integer(1) => 1.0,
            _ => f64::NAN,
        };
        if (0.0..=1.0).contains(&fraction) {
            // From 0 to 100, since the fraction is from 0 to 1.
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok((fraction * 100.0).round() as u8)
        } else {
            Err(self.invalid())
        }
    }

    fn write(&self, value: &u8) -> Option<Value> {
        Some(Value::Float(f64::from(*value) / 100.0))
    }
}

impl Kind for Rgb {
    type Value = Color;

    const EMPTY_UNSETS: bool = true;

    fn expected(&self) -> String {
        "a color like #rrggbb".to_owned()
    }

    fn parse(&self, value: Value, _: Os) -> Result<Color, DiagnosticKind> {
        value
            .as_str()
            .and_then(Color::parse)
            .ok_or_else(|| self.invalid())
    }

    fn write(&self, value: &Color) -> Option<Value> {
        let Color { r, g, b } = value;
        Some(Value::String(format!("#{r:02x}{g:02x}{b:02x}")))
    }
}

impl Kind for Text {
    type Value = String;

    const EMPTY_UNSETS: bool = true;

    fn expected(&self) -> String {
        "a string".to_owned()
    }

    fn parse(&self, value: Value, _: Os) -> Result<String, DiagnosticKind> {
        match value {
            Value::String(text) => Ok(text),
            _ => Err(self.invalid()),
        }
    }

    fn write(&self, value: &String) -> Option<Value> {
        Some(Value::String(value.clone()))
    }
}

impl Kind for Line {
    type Value = CommandLine;

    const EMPTY_UNSETS: bool = true;

    fn expected(&self) -> String {
        Text.expected()
    }

    fn parse(&self, value: Value, os: Os) -> Result<CommandLine, DiagnosticKind> {
        let text = Text.parse(value, os)?;
        CommandLine::parse(&text, os).map_err(DiagnosticKind::InvalidCommandLine)
    }

    fn write(&self, value: &CommandLine) -> Option<Value> {
        Some(Value::String(value.source().to_owned()))
    }
}

impl<K: Kind> Kind for Optional<K> {
    type Value = Option<K::Value>;

    const EMPTY_UNSETS: bool = K::EMPTY_UNSETS;

    fn expected(&self) -> String {
        self.0.expected()
    }

    fn parse(&self, value: Value, os: Os) -> Result<Self::Value, DiagnosticKind> {
        self.0.parse(value, os).map(Some)
    }

    // An item left unset is written as the empty string that unsets it.
    fn write(&self, value: &Self::Value) -> Option<Value> {
        match value {
            Some(value) => self.0.write(value),
            None => K::EMPTY_UNSETS.then(|| Value::String(String::new())),
        }
    }
}

impl Named for SearchMatch {
    const NAMES: &'static [(&'static str, Self)] =
        &[("anywhere", Self::Anywhere), ("start", Self::Start)];
}

impl Named for SizeFormat {
    const NAMES: &'static [(&'static str, Self)] = &[
        ("plain", Self::Plain),
        ("comma", Self::Comma),
        ("human", Self::Human),
    ];
}

impl Named for ImagePlacement {
    const NAMES: &'static [(&'static str, Self)] = &[
        ("other_pane", Self::OtherPane),
        ("both_panes", Self::BothPanes),
    ];
}

impl Named for Theme {
    const NAMES: &'static [(&'static str, Self)] = &[
        ("auto", Self::Auto),
        ("light", Self::Light),
        ("dark", Self::Dark),
    ];
}

const fn empty_unsets<K: Kind>(_: &K) -> bool {
    K::EMPTY_UNSETS
}
