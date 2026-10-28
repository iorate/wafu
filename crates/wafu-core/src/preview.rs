use std::ffi::OsStr;
use std::fmt::Write as _;
use std::path::Path;

use chardetng::{EncodingDetector, Iso2022JpDetection, Utf8Detection};
use encoding_rs::Encoding;

use crate::view::PreviewLine;

/// How much of a file the preview reads.
pub const PREVIEW_LIMIT: u64 = 16 * 1024 * 1024;
/// The line shown after a file cut at [`PREVIEW_LIMIT`].
pub const TRUNCATED_NOTE: &str = "--- truncated: showing the first 16 MiB ---";

const BINARY_PROBE: usize = 8 * 1024;
const MAX_LINE_CHARS: usize = 1024;
const TAB_STOP: usize = 8;
const BYTES_PER_LINE: usize = 16;
const IMAGE_EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "bmp", "webp", "ico"];

/// A file read for the preview, decoded once on the I/O thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub raw: Vec<u8>,
    pub truncated: bool,
    /// The name of the detected encoding, as `encoding_rs` spells it.
    pub encoding: &'static str,
    /// Whether the file reads as text: no NUL in the first 8 KiB and fewer
    /// than one replacement character in a hundred.
    pub is_text: bool,
    /// The decoded lines, kept even for a binary file so that it can be shown as text.
    pub lines: Vec<Line>,
}

/// One decoded line. A long line is cut into several that share its number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Counted from one; zero for the note about truncation.
    pub number: usize,
    pub text: String,
}

/// How a document is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DocumentMode {
    Text,
    Binary,
}

/// A decoded image, with the orientation already applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub format: &'static str,
}

impl Document {
    #[must_use]
    pub fn decode(raw: Vec<u8>, truncated: bool) -> Self {
        // A BOM settles the encoding, and the NUL rule is for files without
        // one: UTF-16 text is full of NUL bytes.
        let (encoding, body, has_nul) = if let Some((encoding, len)) = Encoding::for_bom(&raw) {
            (encoding, &raw[len..], false)
        } else {
            let probe = &raw[..raw.len().min(BINARY_PROBE)];
            let mut detector = EncodingDetector::new(Iso2022JpDetection::Allow);
            detector.feed(&raw, true);
            (
                detector.guess(None, Utf8Detection::Allow),
                &raw[..],
                probe.contains(&0),
            )
        };
        let (text, _) = encoding.decode_without_bom_handling(body);
        let total = text.chars().count();
        let replaced = text.chars().filter(|&c| c == '\u{FFFD}').count();
        let is_text = !has_nul && replaced * 100 < total.max(1);
        let mut lines = split_lines(&text);
        if truncated {
            lines.push(Line {
                number: 0,
                text: TRUNCATED_NOTE.to_owned(),
            });
        }
        Self {
            raw,
            truncated,
            encoding: encoding.name(),
            is_text,
            lines,
        }
    }

    #[must_use]
    pub fn line_count(&self, mode: DocumentMode) -> usize {
        match mode {
            DocumentMode::Text => self.lines.len(),
            DocumentMode::Binary => {
                self.raw.len().div_ceil(BYTES_PER_LINE) + usize::from(self.truncated)
            }
        }
    }

    /// How many digits the widest line number has; zero when no numbers are shown.
    #[must_use]
    pub fn digits(&self, mode: DocumentMode) -> usize {
        match mode {
            DocumentMode::Text => self
                .lines
                .iter()
                .map(|line| line.number)
                .max()
                .map_or(0, |number| number.to_string().len()),
            DocumentMode::Binary => 0,
        }
    }

    /// The lines from `top`, at most `rows` of them. Only the first piece of
    /// a cut line carries its number. The hex dump is made on demand, so that
    /// a large file does not cost a million strings up front.
    #[must_use]
    pub fn lines(&self, mode: DocumentMode, top: usize, rows: usize) -> Vec<PreviewLine> {
        let count = self.line_count(mode);
        let end = top.saturating_add(rows).min(count);
        match mode {
            DocumentMode::Text => (top.min(end)..end)
                .map(|index| {
                    let line = &self.lines[index];
                    let continued = index > 0 && self.lines[index - 1].number == line.number;
                    PreviewLine {
                        number: if continued || line.number == 0 {
                            String::new()
                        } else {
                            line.number.to_string()
                        },
                        text: line.text.clone(),
                    }
                })
                .collect(),
            DocumentMode::Binary => (top.min(end)..end)
                .map(|index| {
                    let text = if self.truncated && index == count - 1 {
                        TRUNCATED_NOTE.to_owned()
                    } else {
                        let offset = index * BYTES_PER_LINE;
                        let end = (offset + BYTES_PER_LINE).min(self.raw.len());
                        hex_line(offset, &self.raw[offset..end])
                    };
                    PreviewLine {
                        number: String::new(),
                        text,
                    }
                })
                .collect(),
        }
    }
}

/// Whether a name has an extension the image preview handles. Case does not matter.
#[must_use]
pub fn is_image_name(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            IMAGE_EXTENSIONS
                .iter()
                .any(|known| known.eq_ignore_ascii_case(extension))
        })
}

// Splits at `\n`, `\r\n`, and `\r`, expands tabs, and cuts lines that would be
// taller than a screen once wrapped. A trailing newline does not add a line.
fn split_lines(text: &str) -> Vec<Line> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut number = 1;
    let mut width = 0;
    // A newline right after a cut would otherwise leave an empty line behind.
    let mut cut = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' || c == '\r' {
            if c == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            if !cut {
                lines.push(Line {
                    number,
                    text: std::mem::take(&mut line),
                });
            }
            number += 1;
            width = 0;
            cut = false;
            continue;
        }
        cut = false;
        if c == '\t' {
            let spaces = TAB_STOP - width % TAB_STOP;
            line.extend(std::iter::repeat_n(' ', spaces));
            width += spaces;
        } else {
            line.push(c);
            width += 1;
        }
        if width >= MAX_LINE_CHARS {
            lines.push(Line {
                number,
                text: std::mem::take(&mut line),
            });
            width = 0;
            cut = true;
        }
    }
    if !line.is_empty() {
        lines.push(Line { number, text: line });
    }
    lines
}

fn hex_line(offset: usize, bytes: &[u8]) -> String {
    let mut line = format!("{offset:08x} ");
    for index in 0..BYTES_PER_LINE {
        if index == BYTES_PER_LINE / 2 {
            line.push(' ');
        }
        match bytes.get(index) {
            Some(byte) => {
                let _ = write!(line, " {byte:02x}");
            }
            None => line.push_str("   "),
        }
    }
    line.push_str("  |");
    for &byte in bytes {
        line.push(if byte.is_ascii_graphic() || byte == b' ' {
            char::from(byte)
        } else {
            '.'
        });
    }
    line.push('|');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_text_is_split_into_lines() {
        let document = Document::decode(b"first\nsecond\n".to_vec(), false);

        assert!(document.is_text);
        assert_eq!(document.encoding, "UTF-8");
        assert_eq!(texts(&document), ["first", "second"]);
        assert_eq!(numbers(&document), [1, 2]);
        assert_eq!(document.line_count(DocumentMode::Text), 2);
        assert_eq!(document.digits(DocumentMode::Text), 1);
        assert_eq!(document.digits(DocumentMode::Binary), 0);
    }

    #[test]
    fn a_nul_in_the_first_8_kib_makes_it_binary() {
        let mut raw = vec![b'a'; 100];
        raw[50] = 0;

        let document = Document::decode(raw, false);

        assert!(!document.is_text);
        assert_eq!(document.lines.len(), 1);
    }

    #[test]
    fn a_nul_after_the_first_8_kib_does_not_count() {
        let mut raw = vec![b'a'; BINARY_PROBE + 1];
        raw[BINARY_PROBE] = 0;

        let document = Document::decode(raw, false);

        assert!(document.is_text);
    }

    #[test]
    fn a_bom_decides_the_encoding() {
        let mut raw = vec![0xff, 0xfe];
        for unit in "hi\n".encode_utf16() {
            raw.extend(unit.to_le_bytes());
        }

        let document = Document::decode(raw, false);

        assert!(document.is_text);
        assert_eq!(document.encoding, "UTF-16LE");
        assert_eq!(texts(&document), ["hi"]);
    }

    #[test]
    fn japanese_in_shift_jis_is_detected() {
        let text = "日本語のテキストファイルです。文字コードは自動で判定します。\n".repeat(4);
        let (raw, _, _) = encoding_rs::SHIFT_JIS.encode(&text);

        let document = Document::decode(raw.into_owned(), false);

        assert!(document.is_text);
        assert_eq!(document.encoding, "Shift_JIS");
        assert_eq!(document.lines.len(), 4);
        assert!(document.lines[0].text.starts_with("日本語の"));
    }

    #[test]
    fn too_many_replacement_characters_make_it_binary() {
        // A UTF-8 BOM forces UTF-8, in which lone continuation bytes decode
        // to replacement characters.
        let mut raw = vec![b'a'; 100];
        for byte in raw.iter_mut().step_by(10) {
            *byte = 0x80;
        }
        raw.splice(0..0, [0xef, 0xbb, 0xbf]);

        let document = Document::decode(raw, false);

        assert!(!document.is_text);
    }

    #[test]
    fn carriage_returns_and_tabs_are_normalized() {
        let document = Document::decode(b"a\r\nb\rc\td\n\tx".to_vec(), false);

        assert_eq!(texts(&document), ["a", "b", "c       d", "        x"]);
    }

    #[test]
    fn empty_input_has_no_lines_and_a_trailing_newline_adds_none() {
        assert_eq!(
            texts(&Document::decode(Vec::new(), false)),
            Vec::<String>::new()
        );
        assert_eq!(texts(&Document::decode(b"\n".to_vec(), false)), [""]);
        assert_eq!(
            texts(&Document::decode(b"a\n\n".to_vec(), false)),
            ["a", ""]
        );
    }

    #[test]
    fn long_lines_are_cut_without_leaving_empty_lines() {
        let long = "x".repeat(MAX_LINE_CHARS + 1);
        let exact = "y".repeat(MAX_LINE_CHARS);

        let document = Document::decode(format!("{long}\n{exact}\nz").into_bytes(), false);

        assert_eq!(
            texts(&document),
            [
                "x".repeat(MAX_LINE_CHARS),
                "x".to_owned(),
                exact,
                "z".to_owned()
            ]
        );
        assert_eq!(numbers(&document), [1, 1, 2, 3]);
        assert_eq!(
            document
                .lines(DocumentMode::Text, 1, 3)
                .iter()
                .map(|line| line.number.as_str())
                .collect::<Vec<_>>(),
            ["", "2", "3"]
        );
    }

    #[test]
    fn a_truncated_document_ends_with_the_note() {
        let document = Document::decode(vec![b'a'; 20], true);

        assert_eq!(
            texts(&document),
            ["a".repeat(20), TRUNCATED_NOTE.to_owned()]
        );
        assert_eq!(numbers(&document), [1, 0]);
        assert_eq!(document.line_count(DocumentMode::Binary), 3);
        assert_eq!(
            shown(&document, DocumentMode::Binary, 2, 5),
            [TRUNCATED_NOTE.to_owned()]
        );
        assert_eq!(
            document.lines(DocumentMode::Text, 1, 1)[0].number,
            String::new()
        );
    }

    #[test]
    fn hex_lines_follow_the_hexdump_layout() {
        let document = Document::decode(b"Hello world\n\x00\x01\x7f\xffabc".to_vec(), false);

        assert_eq!(document.line_count(DocumentMode::Binary), 2);
        assert_eq!(
            shown(&document, DocumentMode::Binary, 0, 10),
            [
                "00000000  48 65 6c 6c 6f 20 77 6f  72 6c 64 0a 00 01 7f ff  |Hello world.....|",
                "00000010  61 62 63                                          |abc|",
            ]
        );
        assert_eq!(document.lines(DocumentMode::Binary, 1, 1).len(), 1);
        assert_eq!(document.lines(DocumentMode::Binary, 5, 1).len(), 0);
    }

    #[test]
    fn text_lines_are_taken_from_the_top_within_bounds() {
        let document = Document::decode(b"a\nb\nc\n".to_vec(), false);

        assert_eq!(shown(&document, DocumentMode::Text, 1, 5), ["b", "c"]);
        assert_eq!(
            shown(&document, DocumentMode::Text, 9, 5),
            Vec::<String>::new()
        );
    }

    #[test]
    fn digits_follow_the_last_line_number() {
        let text = "x\n".repeat(120);

        let document = Document::decode(text.into_bytes(), false);

        assert_eq!(document.digits(DocumentMode::Text), 3);
        assert_eq!(document.lines(DocumentMode::Text, 99, 1)[0].number, "100");
    }

    fn texts(document: &Document) -> Vec<String> {
        document
            .lines
            .iter()
            .map(|line| line.text.clone())
            .collect()
    }

    fn numbers(document: &Document) -> Vec<usize> {
        document.lines.iter().map(|line| line.number).collect()
    }

    fn shown(document: &Document, mode: DocumentMode, top: usize, rows: usize) -> Vec<String> {
        document
            .lines(mode, top, rows)
            .into_iter()
            .map(|line| line.text)
            .collect()
    }

    #[test]
    fn image_names_are_recognized_by_extension() {
        assert!(is_image_name(OsStr::new("a.png")));
        assert!(is_image_name(OsStr::new("A.JPG")));
        assert!(is_image_name(OsStr::new("photo.jpeg")));
        assert!(is_image_name(OsStr::new("a.webp")));
        assert!(!is_image_name(OsStr::new("a.txt")));
        assert!(!is_image_name(OsStr::new(".png")));
        assert!(!is_image_name(OsStr::new("png")));
    }
}
