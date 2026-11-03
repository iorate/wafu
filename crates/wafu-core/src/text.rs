use std::borrow::Cow;

use icu_normalizer::ComposingNormalizer;

// macOS stores file names decomposed (NFD) while input methods produce composed
// text (NFC), so both sides are composed before any name comparison.
pub(crate) fn nfc(text: &str) -> Cow<'_, str> {
    ComposingNormalizer::new_nfc().normalize(text)
}
