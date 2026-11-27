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
// fixes the type of the value, and the closure that stores it is checked
// against the field's type.
macro_rules! setting {
    ($key:literal, $kind:expr, $($field:tt)+) => {
        Setting {
            key: $key,
            empty_unsets: empty_unsets(&$kind),
            read: |target, value, os| {
                target.$($field)+ = Kind::parse(&$kind, value, os)?;
                Ok(())
            },
        }
    };
}

pub(super) const FILELIST: &[Setting<FileListConfig>] = &[
    setting!("new_dir_goes_to_dir", Bool, new_dir_goes_to_dir),
    setting!("new_text_creates_file", Bool, new_text_creates_file),
    setting!("extract_creates_dir", Bool, extract_creates_dir),
    setting!("require_marks", Bool, require_marks),
    setting!("search_match", Choice::<SearchMatch>::NEW, search_match),
    setting!("time_shows_full_year", Bool, time_format.full_year),
    setting!("time_shows_seconds", Bool, time_format.seconds),
    setting!("size_format", Choice::<SizeFormat>::NEW, size_format),
    setting!(
        "size_digits",
        Count {
            min: SIZE_DIGITS_MIN,
            max: SIZE_DIGITS_MAX
        },
        size_digits
    ),
    setting!("extension_chars", COLUMN_CHARS, extension_chars),
    setting!("min_name_chars", COLUMN_CHARS, min_name_chars),
    setting!("max_history", Limit, max_history),
    setting!("max_find_matches", Limit, max_find_matches),
    setting!(
        "mark_compare_tolerance_ms",
        NonNegative,
        mark_compare_tolerance_ms
    ),
];

pub(super) const TEXT_PREVIEW: &[Setting<TextPreviewConfig>] = &[
    setting!("hide_log", Bool, hide_log),
    setting!("max_bytes", Limit, max_bytes),
];

pub(super) const IMAGE_PREVIEW: &[Setting<ImagePreviewConfig>] = &[
    setting!(
        "placement_from_left",
        Choice::<ImagePlacement>::NEW,
        placement_from_left
    ),
    setting!(
        "placement_from_right",
        Choice::<ImagePlacement>::NEW,
        placement_from_right
    ),
    setting!("hide_log", Bool, hide_log),
    setting!("max_bytes", Limit, max_bytes),
];

pub(super) const LOG: &[Setting<LogConfig>] = &[setting!("max_lines", Limit, max_lines)];

pub(super) const PROGRAMS: &[Setting<ProgramsConfig>] = &[
    setting!("editor", Line, editor),
    setting!("use_login_shell_env", Bool, use_login_shell_env),
];

pub(super) const FONT: &[Setting<Font>] = &[
    setting!("family", Optional(Text), family),
    setting!("size", Optional(FontSize), size),
];

pub(super) const COLORS_CONFIG: &[Setting<ColorsConfig>] = &[
    setting!("theme", Choice::<Theme>::NEW, theme),
    setting!("hidden_opacity", Opacity, hidden_opacity),
];

pub(super) const COLORS: &[Setting<Colors>] = &[
    setting!("fg", Rgb, fg),
    setting!("bg", Rgb, bg),
    setting!("line_number", Rgb, line_number),
    setting!("mark", Rgb, mark),
    setting!("cursor", Rgb, cursor),
    setting!("cursor_inactive", Rgb, cursor_inactive),
    setting!("dir", Rgb, dir),
    setting!("link", Rgb, link),
    setting!("exec", Rgb, exec),
    setting!("link_broken", Rgb, link_broken),
    setting!("special", Rgb, special),
    setting!("socket", Rgb, socket),
];

const COLUMN_CHARS: Count = Count {
    min: 1,
    max: COLUMN_CHARS_MAX,
};

pub(super) struct Setting<T> {
    pub(super) key: &'static str,
    pub(super) empty_unsets: bool,
    pub(super) read: fn(&mut T, Value, Os) -> Result<(), DiagnosticKind>,
}

pub(super) trait Kind {
    type Value;

    // Only for the kinds written as strings: an empty string is no value of
    // theirs, while the others have no such spare value.
    const EMPTY_UNSETS: bool = false;

    fn expected(&self) -> String;

    fn parse(&self, value: Value, os: Os) -> Result<Self::Value, DiagnosticKind>;

    fn invalid(&self) -> DiagnosticKind {
        DiagnosticKind::InvalidValue {
            expected: self.expected(),
        }
    }
}

pub(super) trait Named: Copy + 'static {
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
