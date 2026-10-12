use std::borrow::Cow;

use icu_normalizer::ComposingNormalizer;

// macOS stores file names decomposed (NFD) while input methods produce composed
// text (NFC), so both sides are composed before any name comparison.
pub(crate) fn nfc(text: &str) -> Cow<'_, str> {
    ComposingNormalizer::new_nfc().normalize(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nfc_composes_decomposed_kana() {
        assert_eq!(nfc("\u{30d5}\u{309a}"), "\u{30d7}");
        assert_eq!(nfc("abc"), "abc");
    }
}
