use std::borrow::Cow;

use icu_normalizer::ComposingNormalizer;

use crate::app::Os;

pub(crate) const LINE_BREAK_SYMBOL: &str = "\u{21B5}";
const RESERVED_NAMES: [&str; 6] = ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"];
const NUMBERED_RESERVED_NAMES: [&str; 2] = ["COM", "LPT"];

/// Why a name cannot be used as one path element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProblem {
    /// Empty, `.`, `..`, or with a NUL character.
    Invalid,
    /// With a separator, or on Windows a `:`, which starts a drive or a stream.
    Separator,
    /// With a character Windows does not allow.
    Character,
    /// Ending with a dot or a space, which Windows drops.
    Ending,
    /// A name Windows reserves for a device.
    Device,
}

impl NameProblem {
    pub(crate) fn reason(self) -> &'static str {
        match self {
            Self::Invalid => "invalid name",
            Self::Separator => "contains a separator",
            Self::Character => "contains a character that Windows does not allow",
            Self::Ending => "ends with a dot or a space, which Windows does not allow",
            Self::Device => "reserved for a device on Windows",
        }
    }
}

/// Composes `text` (NFC).
// macOS stores file names decomposed (NFD) while input methods produce composed
// text (NFC), so both sides are composed before any name comparison.
#[must_use]
pub fn nfc(text: &str) -> Cow<'_, str> {
    ComposingNormalizer::new_nfc().normalize(text)
}

/// Whether `name` can be used as one path element on `os`, with the problem
/// if not.
#[must_use]
pub fn name_problem(name: &str, os: Os) -> Option<NameProblem> {
    let windows = os == Os::Windows;
    if name.is_empty() || name == "." || name == ".." || name.contains('\0') {
        Some(NameProblem::Invalid)
    } else if name.contains('/') || (windows && name.contains(['\\', ':'])) {
        Some(NameProblem::Separator)
    } else if !windows {
        None
    } else if name.contains(|c: char| {
        // Only these controls: DEL and the C1 controls are allowed.
        ('\u{1}'..='\u{1f}').contains(&c) || "<>\"|?*".contains(c)
    }) {
        Some(NameProblem::Character)
    } else if name.ends_with(['.', ' ']) {
        Some(NameProblem::Ending)
    } else if is_reserved(name) {
        Some(NameProblem::Device)
    } else {
        None
    }
}

// The case counts: whether the file system tells it apart is not known here.
pub(crate) fn same_name(a: &str, b: &str) -> bool {
    nfc(a) == nfc(b)
}

// The text layout forces a line break after these characters, so a single-row
// text holding one is laid out as two lines: the first is pushed up and cut
// short, and the rest is not shown at all. A file name may contain them (the
// macOS `Icon\r`), so they are shown as a symbol instead.
pub(crate) fn show_line_breaks(text: String) -> String {
    if !text.contains(is_line_break) {
        return text;
    }
    // CR+LF ends one line, not two.
    text.replace("\r\n", "\r")
        .replace(is_line_break, LINE_BREAK_SYMBOL)
}

pub(crate) fn is_line_break(c: char) -> bool {
    matches!(
        c,
        '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

// `CON`, `COM1`, and the like name devices on Windows even with an extension
// and with spaces before it. The superscript digits count as digits there.
fn is_reserved(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches(' ');
    RESERVED_NAMES
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
        || NUMBERED_RESERVED_NAMES.iter().any(|reserved| {
            stem.get(..reserved.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(reserved))
                && stem.get(reserved.len()..).is_some_and(|number| {
                    let mut chars = number.chars();
                    chars
                        .next()
                        .is_some_and(|c| matches!(c, '1'..='9' | '\u{b9}' | '\u{b2}' | '\u{b3}'))
                        && chars.next().is_none()
                })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_name_ignores_normalization_but_not_case() {
        assert!(same_name("\u{304b}\u{3099}.txt", "\u{304c}.txt"));
        assert!(!same_name("\u{304b}.txt", "\u{304c}.txt"));
        assert!(!same_name("a.txt", "A.txt"));
    }

    #[test]
    fn name_problem_follows_the_rules_of_each_os() {
        use NameProblem::{Character, Device, Ending, Invalid, Separator};

        let cases = [
            ("a.txt", None, None),
            ("...", None, Some(Ending)),
            ("", Some(Invalid), Some(Invalid)),
            (".", Some(Invalid), Some(Invalid)),
            ("..", Some(Invalid), Some(Invalid)),
            ("a\0b", Some(Invalid), Some(Invalid)),
            ("a/b", Some(Separator), Some(Separator)),
            ("a\\b", None, Some(Separator)),
            ("c:x", None, Some(Separator)),
            ("a.txt:s", None, Some(Separator)),
            ("a?b", None, Some(Character)),
            ("a<b", None, Some(Character)),
            ("a>b", None, Some(Character)),
            ("a\"b", None, Some(Character)),
            ("a|b", None, Some(Character)),
            ("a*b", None, Some(Character)),
            ("a\u{1}b", None, Some(Character)),
            ("a\u{1f}b", None, Some(Character)),
            ("a\u{7f}b", None, None),
            ("a\u{80}b", None, None),
            ("x.", None, Some(Ending)),
            ("x ", None, Some(Ending)),
            ("con", None, Some(Device)),
            ("CON.txt", None, Some(Device)),
            ("nul.tar.gz", None, Some(Device)),
            ("con .txt", None, Some(Device)),
            ("CONIN$", None, Some(Device)),
            ("com1", None, Some(Device)),
            ("LPT9", None, Some(Device)),
            ("com\u{b9}", None, Some(Device)),
            ("console", None, None),
            (" con", None, None),
            ("com0", None, None),
            ("com10", None, None),
            ("lpt", None, None),
            ("com\u{2074}", None, None),
        ];
        for (name, unix, windows) in cases {
            assert_eq!(name_problem(name, Os::MacOs), unix, "{name:?} on macOS");
            assert_eq!(name_problem(name, Os::Linux), unix, "{name:?} on Linux");
            assert_eq!(name_problem(name, Os::Windows), windows, "{name:?}");
        }
    }

    #[test]
    fn show_line_breaks_replaces_each_line_break_with_the_symbol() {
        let cases = [
            ("\n", "\u{21B5}"),
            ("\u{0B}", "\u{21B5}"),
            ("\u{0C}", "\u{21B5}"),
            ("\r", "\u{21B5}"),
            ("\u{85}", "\u{21B5}"),
            ("\u{2028}", "\u{21B5}"),
            ("\u{2029}", "\u{21B5}"),
            ("\r\n", "\u{21B5}"),
            ("\n\r", "\u{21B5}\u{21B5}"),
            ("\r\u{2028}", "\u{21B5}\u{21B5}"),
        ];
        for (text, expected) in cases {
            assert_eq!(show_line_breaks(text.to_owned()), expected, "{text:?}");
        }
    }

    #[test]
    fn show_line_breaks_keeps_other_characters() {
        assert_eq!(
            show_line_breaks(
                "Icon\r\nメモ \t\u{1}\u{0E}\u{84}\u{86}\u{2027}\u{202A}.txt".to_owned()
            ),
            "Icon\u{21B5}メモ \t\u{1}\u{0E}\u{84}\u{86}\u{2027}\u{202A}.txt"
        );
    }
}
