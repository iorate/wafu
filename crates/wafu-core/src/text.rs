use std::borrow::Cow;

use icu_normalizer::ComposingNormalizer;

pub(crate) const LINE_BREAK_SYMBOL: &str = "\u{21B5}";

// macOS stores file names decomposed (NFD) while input methods produce composed
// text (NFC), so both sides are composed before any name comparison.
pub(crate) fn nfc(text: &str) -> Cow<'_, str> {
    ComposingNormalizer::new_nfc().normalize(text)
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

#[cfg(test)]
mod tests {
    use super::*;

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
