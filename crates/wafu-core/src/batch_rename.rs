use std::collections::HashSet;
use std::fmt::Write;

use thiserror::Error;

use crate::app::Os;
use crate::entry::Entry;
use crate::text::nfc;

/// What the new names of a batch rename are made from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenameInput<'a> {
    /// Every occurrence of `find` in the whole name, ignoring case, becomes
    /// `replace` as written.
    Replace { find: &'a str, replace: &'a str },
    /// A template of the whole name, with `{name}`, `{ext}`, and counters
    /// such as `{001}`.
    Template(&'a str),
}

/// The new names of a batch rename, and the first reason it cannot run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenamePlan {
    /// The new name of each target, in the order given, or `None` when its
    /// name does not change.
    pub names: Vec<Option<String>>,
    /// The first error, as the note shows it.
    pub error: Option<String>,
}

impl RenamePlan {
    /// Plans renaming `targets`, given in display order. `existing` is the
    /// pane's whole listing, hidden and filtered entries included, or `None`
    /// when the listing does not hold the targets' siblings.
    #[must_use]
    pub fn new(
        targets: &[Entry],
        input: RenameInput<'_>,
        existing: Option<&[Entry]>,
        os: Os,
    ) -> Self {
        let new_names: Vec<String> = match input {
            RenameInput::Replace { find, replace } => targets
                .iter()
                .map(|target| replace_in(&target.name.to_string_lossy(), find, replace))
                .collect(),
            RenameInput::Template(text) => match parse_template(text) {
                Ok(template) => targets
                    .iter()
                    .enumerate()
                    .map(|(index, target)| apply_template(&template, target, index))
                    .collect(),
                Err(error) => {
                    return Self {
                        names: vec![None; targets.len()],
                        error: Some(error.to_string()),
                    };
                }
            },
        };
        let names: Vec<Option<String>> = targets
            .iter()
            .zip(new_names)
            .map(|(target, new)| (new != nfc(&target.name.to_string_lossy())).then_some(new))
            .collect();
        let error = first_error(targets, &names, existing, os);
        Self { names, error }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    Name,
    Ext,
    Counter { start: u64, width: usize },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
enum TemplateError {
    #[error("Unknown placeholder {{{0}}}")]
    UnknownPlaceholder(String),
    #[error("Unclosed {{")]
    Unclosed,
    #[error("Unmatched }}")]
    Unmatched,
}

fn replace_in(name: &str, find: &str, replace: &str) -> String {
    let name = nfc(name);
    let needle = lowercase(&nfc(find));
    if needle.is_empty() {
        return name.into_owned();
    }
    let mut renamed = String::new();
    let mut rest = name.as_ref();
    while let Some(c) = rest.chars().next() {
        if let Some(len) = match_len(rest, &needle) {
            renamed.push_str(replace);
            rest = &rest[len..];
        } else {
            renamed.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    nfc(&renamed).into_owned()
}

// Each character is lowercased on its own, and a match must end where a
// character of `text` ends: lowercasing changes the length of some characters
// (`İ`), so positions in the lowercased text do not point into `text`.
fn match_len(text: &str, needle: &str) -> Option<usize> {
    let mut rest = needle;
    for (index, c) in text.char_indices() {
        for lower in c.to_lowercase() {
            rest = rest.strip_prefix(lower)?;
        }
        if rest.is_empty() {
            return Some(index + c.len_utf8());
        }
    }
    None
}

fn lowercase(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

fn parse_template(text: &str) -> Result<Vec<Piece>, TemplateError> {
    let mut pieces = Vec::new();
    let mut literal = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        rest = &rest[c.len_utf8()..];
        match c {
            '{' => {
                if let Some(after) = rest.strip_prefix('{') {
                    literal.push('{');
                    rest = after;
                    continue;
                }
                let (inside, after) = rest.split_once('}').ok_or(TemplateError::Unclosed)?;
                rest = after;
                if !literal.is_empty() {
                    pieces.push(Piece::Text(std::mem::take(&mut literal)));
                }
                pieces.push(placeholder(inside)?);
            }
            '}' => {
                rest = rest.strip_prefix('}').ok_or(TemplateError::Unmatched)?;
                literal.push('}');
            }
            _ => literal.push(c),
        }
    }
    if !literal.is_empty() {
        pieces.push(Piece::Text(literal));
    }
    Ok(pieces)
}

fn placeholder(inside: &str) -> Result<Piece, TemplateError> {
    match inside {
        "name" => Ok(Piece::Name),
        "ext" => Ok(Piece::Ext),
        // Eighteen digits always fit in a `u64`, with room left for counting up.
        _ if (1..=18).contains(&inside.len()) && inside.bytes().all(|b| b.is_ascii_digit()) => {
            Ok(Piece::Counter {
                start: inside.parse().expect("ASCII digits fit in a u64"),
                width: inside.len(),
            })
        }
        _ => Err(TemplateError::UnknownPlaceholder(inside.to_owned())),
    }
}

fn apply_template(template: &[Piece], target: &Entry, index: usize) -> String {
    let name = target.name.to_string_lossy();
    // Split the same way `rename` chooses what to select: a name it selects
    // whole (a directory, `.gitignore`, `a.`) has no extension.
    let (stem, extension) = target.split_name();
    let (stem, extension) = if extension.is_empty() {
        (name.as_ref(), "")
    } else {
        (&name[..stem.len()], &name[stem.len()..])
    };
    let mut renamed = String::new();
    for piece in template {
        match piece {
            Piece::Text(text) => renamed.push_str(text),
            Piece::Name => renamed.push_str(stem),
            Piece::Ext => renamed.push_str(extension),
            Piece::Counter { start, width } => {
                let number = start + index as u64;
                write!(renamed, "{number:0width$}").expect("writing to a String cannot fail");
            }
        }
    }
    nfc(&renamed).into_owned()
}

fn first_error(
    targets: &[Entry],
    names: &[Option<String>],
    existing: Option<&[Entry]>,
    os: Os,
) -> Option<String> {
    let renamed: Vec<(&Entry, &str)> = targets
        .iter()
        .zip(names)
        .filter_map(|(target, name)| Some((target, name.as_deref()?)))
        .collect();
    let old_names: HashSet<String> = renamed
        .iter()
        .map(|(target, _)| nfc(&target.name.to_string_lossy()).into_owned())
        .collect();
    // A renamed item moves away from its old name, so only the others stay in
    // the way. A name that is not UTF-8 never equals a new name, and a lossy
    // conversion could make it look as if it did.
    let staying: HashSet<String> = existing
        .unwrap_or_default()
        .iter()
        .filter_map(|entry| entry.name.to_str())
        .map(|name| nfc(name).into_owned())
        .filter(|name| !old_names.contains(name))
        .collect();
    let mut taken = HashSet::new();
    renamed.into_iter().find_map(|(target, new)| {
        let reason = if new.is_empty() {
            "empty name"
        } else if new == "." || new == ".." {
            "invalid name"
        } else if new.contains('/') || (os == Os::Windows && new.contains('\\')) {
            "contains a separator"
        } else if !taken.insert(lowercase(new)) {
            // Creating `A` and `a` at once fails halfway on a file system that
            // ignores case, so they are refused everywhere.
            "duplicate name"
        } else if staying.contains(new) {
            "already exists"
        } else {
            return None;
        };
        Some(format!(
            "{} \u{2192} {new}: {reason}",
            target.name.to_string_lossy()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{EntryKind, TargetKind};
    use crate::test_support::{dir, file};

    #[test]
    fn replace_replaces_every_occurrence_ignoring_case_and_inserts_the_replacement_as_written() {
        assert_eq!(
            replace_in("IMG_a_img_.JPG", "img_", "Trip_"),
            "Trip_a_Trip_.JPG"
        );
    }

    #[test]
    fn replace_matches_from_the_left_without_overlap_or_searching_the_replacement() {
        assert_eq!(replace_in("aaa", "aa", "b"), "ba");
        assert_eq!(replace_in("a", "a", "aa"), "aa");
    }

    #[test]
    fn replace_with_an_empty_find_changes_nothing() {
        assert_eq!(replace_in("a", "", "x"), "a");
    }

    #[test]
    fn replace_matches_on_character_boundaries_when_lowercasing_changes_the_length() {
        assert_eq!(replace_in("\u{130}x", "i", "y"), "\u{130}x");
        assert_eq!(replace_in("\u{130}ab", "ab", "c"), "\u{130}c");
        assert_eq!(replace_in("\u{130}", "i\u{307}", "y"), "y");
    }

    #[test]
    fn replace_lowercases_each_character_on_its_own() {
        assert_eq!(replace_in("\u{391}\u{3a3}", "\u{3c3}", "x"), "\u{391}x");
    }

    #[test]
    fn replace_composes_the_name_and_find_before_matching() {
        assert_eq!(replace_in("ca\u{301}fe\u{301}", "\u{e9}", "e"), "c\u{e1}fe");
        assert_eq!(replace_in("\u{e9}", "e\u{301}", "x"), "x");
    }

    #[test]
    fn replace_composes_the_result() {
        assert_eq!(replace_in("x\u{301}", "x", "e"), "\u{e9}");
    }

    #[test]
    fn parse_template_rejects_bad_formats() {
        let cases = [
            ("{x}", "Unknown placeholder {x}"),
            ("{NAME}", "Unknown placeholder {NAME}"),
            ("{ name }", "Unknown placeholder { name }"),
            ("{}", "Unknown placeholder {}"),
            ("{-1}", "Unknown placeholder {-1}"),
            (
                "{1234567890123456789}",
                "Unknown placeholder {1234567890123456789}",
            ),
            ("{a{b}", "Unknown placeholder {a{b}"),
            ("{name", "Unclosed {"),
            ("a}", "Unmatched }"),
            ("{{name}", "Unmatched }"),
            ("}{", "Unmatched }"),
        ];
        for (text, expected) in cases {
            let error = parse_template(text).expect_err(text);
            assert_eq!(error.to_string(), expected, "{text:?}");
        }
    }

    #[test]
    fn template_reads_escaped_braces_around_a_placeholder() {
        assert_eq!(apply("{{{name}}}", &file("a.txt"), 0), "{a}");
        assert_eq!(apply("{{name}}", &file("a.txt"), 0), "{name}");
    }

    #[test]
    fn template_splits_the_name_as_rename_selects_it() {
        let link = |name, target| Entry {
            kind: EntryKind::Symlink { target },
            ..file(name)
        };
        let cases = [
            (file("a.txt"), "<a|.txt>"),
            (file("a.tar.gz"), "<a.tar|.gz>"),
            (file(".a.b"), "<.a|.b>"),
            (file(".gitignore"), "<.gitignore|>"),
            (file("a."), "<a.|>"),
            (file("a"), "<a|>"),
            (dir("a.d"), "<a.d|>"),
            (link("l.d", Some(TargetKind::Dir)), "<l.d|>"),
            (
                link("l.txt", Some(TargetKind::File { executable: false })),
                "<l|.txt>",
            ),
        ];
        for (entry, expected) in cases {
            assert_eq!(apply("<{name}|{ext}>", &entry, 0), expected, "{entry:?}");
        }
    }

    #[test]
    fn template_name_and_ext_give_the_composed_original() {
        assert_eq!(
            apply("{name}{ext}", &file("e\u{301}.t\u{30c}"), 0),
            "\u{e9}.\u{165}"
        );
    }

    #[test]
    fn template_does_not_read_the_name_again() {
        assert_eq!(apply("{name}", &file("{ext}.txt"), 0), "{ext}");
    }

    #[test]
    fn template_composes_the_result() {
        assert_eq!(apply("e\u{301}{ext}", &file("a.txt"), 0), "\u{e9}.txt");
    }

    #[test]
    fn template_counters_count_up_from_their_start_padded_to_their_width() {
        let cases = [
            ("{1}", 0, "1"),
            ("{1}", 1, "2"),
            ("{001}", 0, "001"),
            ("{010}", 1, "011"),
            ("{01}", 99, "100"),
            ("{1}-{100}", 2, "3-102"),
            ("{999999999999999999}", 0, "999999999999999999"),
        ];
        for (text, index, expected) in cases {
            assert_eq!(apply(text, &file("a"), index), expected, "{text:?} {index}");
        }
    }

    #[test]
    fn plan_leaves_out_targets_whose_composed_name_does_not_change() {
        let targets = [file("x"), file("e\u{301}")];
        let plan = RenamePlan::new(&targets, replace("x", "y"), Some(&targets), Os::Linux);
        assert_eq!(plan.names, [Some("y".to_owned()), None]);
        assert_eq!(plan.error, None);
    }

    #[test]
    fn plan_reports_a_template_format_error_without_new_names() {
        let targets = [file("a")];
        let plan = RenamePlan::new(&targets, RenameInput::Template("{name"), None, Os::Linux);
        assert_eq!(plan.names, [None]);
        assert_eq!(plan.error.as_deref(), Some("Unclosed {"));
    }

    #[test]
    fn plan_rejects_names_that_cannot_be_made() {
        let cases = [
            (replace("a", ""), Os::Linux, "a \u{2192} : empty name"),
            (
                RenameInput::Template("."),
                Os::Linux,
                "a \u{2192} .: invalid name",
            ),
            (
                RenameInput::Template(".."),
                Os::Linux,
                "a \u{2192} ..: invalid name",
            ),
            (
                RenameInput::Template("b/c"),
                Os::Linux,
                "a \u{2192} b/c: contains a separator",
            ),
            (
                RenameInput::Template("b\\c"),
                Os::Windows,
                "a \u{2192} b\\c: contains a separator",
            ),
        ];
        for (input, os, expected) in cases {
            let plan = RenamePlan::new(&[file("a")], input, None, os);
            assert_eq!(plan.error.as_deref(), Some(expected), "{input:?}");
        }
    }

    #[test]
    fn plan_allows_a_backslash_outside_windows() {
        let plan = RenamePlan::new(&[file("a")], RenameInput::Template("b\\c"), None, Os::Linux);
        assert_eq!(plan.error, None);
    }

    #[test]
    fn plan_rejects_new_names_equal_ignoring_case_and_normalization() {
        let cases = [
            (
                [file("A.txt"), file("a.jpg")],
                "{name}",
                "a.jpg \u{2192} a: duplicate name",
            ),
            (
                [file("e\u{301}.txt"), file("\u{e9}.jpg")],
                "{name}",
                "\u{e9}.jpg \u{2192} \u{e9}: duplicate name",
            ),
            (
                [file("a\u{3a3}.txt"), file("a\u{3c3}.jpg")],
                "{name}",
                "a\u{3c3}.jpg \u{2192} a\u{3c3}: duplicate name",
            ),
        ];
        for (targets, template, expected) in cases {
            let plan = RenamePlan::new(&targets, RenameInput::Template(template), None, Os::Linux);
            assert_eq!(plan.error.as_deref(), Some(expected));
        }
    }

    #[test]
    fn plan_rejects_an_existing_name_by_normalization_but_not_case() {
        let targets = [file("a.txt")];
        let listing = [file("a.txt"), file("A.png"), file("e\u{301}.png")];
        let error = |template| {
            RenamePlan::new(
                &targets,
                RenameInput::Template(template),
                Some(&listing),
                Os::Linux,
            )
            .error
        };
        assert_eq!(error("a.png"), None);
        assert_eq!(
            error("\u{e9}.png").as_deref(),
            Some("a.txt \u{2192} \u{e9}.png: already exists")
        );
    }

    #[test]
    fn plan_frees_the_old_names_of_renamed_items() {
        let targets = [file("1"), file("2")];
        let plan = RenamePlan::new(
            &targets,
            RenameInput::Template("{2}"),
            Some(&targets),
            Os::Linux,
        );
        assert_eq!(plan.names, [Some("2".to_owned()), Some("3".to_owned())]);
        assert_eq!(plan.error, None);
    }

    #[test]
    fn plan_keeps_unchanged_targets_in_the_way() {
        let targets = [file("a"), file("b")];
        let plan = RenamePlan::new(&targets, replace("b", "a"), Some(&targets), Os::Linux);
        assert_eq!(plan.error.as_deref(), Some("b \u{2192} a: already exists"));
    }

    #[test]
    fn plan_reports_the_first_item_in_order_with_a_problem() {
        let targets = [file("ab"), file("b")];
        let listing = [file("ab"), file("b"), file("a")];
        let plan = RenamePlan::new(&targets, replace("b", ""), Some(&listing), Os::Linux);
        assert_eq!(plan.error.as_deref(), Some("ab \u{2192} a: already exists"));
    }

    #[test]
    fn plan_reports_a_duplicate_before_an_existing_name() {
        let targets = [file("Xa"), file("xa")];
        let listing = [file("Xa"), file("xa"), file("x")];
        let plan = RenamePlan::new(&targets, replace("a", ""), Some(&listing), Os::Linux);
        assert_eq!(plan.error.as_deref(), Some("xa \u{2192} x: duplicate name"));
    }

    #[cfg(unix)]
    #[test]
    fn plan_does_not_match_an_existing_name_that_is_not_utf8() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let targets = [file("a")];
        let listing = [
            file("a"),
            Entry {
                name: OsString::from_vec(b"\xff".to_vec()),
                ..file("")
            },
        ];
        let plan = RenamePlan::new(
            &targets,
            RenameInput::Template("\u{fffd}"),
            Some(&listing),
            Os::Linux,
        );
        assert_eq!(plan.error, None);
    }

    fn apply(template: &str, target: &Entry, index: usize) -> String {
        apply_template(&parse_template(template).unwrap(), target, index)
    }

    fn replace<'a>(find: &'a str, replace: &'a str) -> RenameInput<'a> {
        RenameInput::Replace { find, replace }
    }
}
