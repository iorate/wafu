use std::cmp::Reverse;
use std::fmt::Write;

use crate::app::Os;
use crate::command::{Command, Context};
use crate::keymap::KeyMap;

const HEADER: &str = "\
<!-- Generated from the command table of wafu-core. Do not edit by hand; regenerate with: \
WAFU_UPDATE_DOCS=1 cargo test -p wafu-core --test commands_doc -->

# Commands

To bind keys to these commands, see [Keys](configuration.md#keys). \
A command works only in the file list unless its section or its description says otherwise.
";

// Each section runs from its first command up to the next section's, so
// that a new command falls into the section it is listed in.
const SECTIONS: &[(&str, Command)] = &[
    ("Opening", Command::Open),
    ("Navigation", Command::GoParent),
    ("Panes", Command::FocusLeft),
    ("Cursor and marks", Command::CursorUp),
    ("View", Command::Sort),
    ("Log", Command::LogShrink),
    ("Preview", Command::PreviewScrollUp),
    ("Files", Command::Copy),
    ("Archives", Command::OpenAsArchive),
    ("Application", Command::Palette),
];

const CONTEXTS: [(Context, &str); 3] = [
    (Context::FileList, "file list"),
    (Context::Image, "image preview"),
    (Context::Text, "text preview"),
];

const FILE_LIST_ONLY: Contexts = [true, false, false];

const JUMP_CLASSES: [(char, char); 2] = [('a', 'z'), ('0', '9')];

// Whether a command works in each of `CONTEXTS`, in that order.
type Contexts = [bool; 3];

struct Row {
    name: String,
    keys: String,
    description: &'static str,
    contexts: Contexts,
    windows_only: bool,
}

/// The Markdown of `docs/commands.md`: every built-in command with its
/// default keys and description. The same on every OS.
#[must_use]
pub fn commands_doc() -> String {
    let keymap = KeyMap::default_for(Os::Windows);
    let mut sections: Vec<(&str, Vec<Row>)> = Vec::new();
    for command in Command::all() {
        if let Some((heading, _)) = SECTIONS.iter().find(|(_, start)| *start == command) {
            sections.push((*heading, Vec::new()));
        }
        let (name, keys) = match command {
            Command::Jump(c) if c != JUMP_CLASSES[0].0 => continue,
            Command::Jump(_) => jump_cells(),
            _ => (
                code_span(&command.to_string()),
                command_keys(&keymap, &command),
            ),
        };
        let row = Row {
            name,
            keys,
            description: command.description().unwrap_or_default(),
            contexts: contexts_of(&command),
            windows_only: !command.available_on(Os::Linux),
        };
        if let Some((_, rows)) = sections.last_mut() {
            rows.push(row);
        }
    }
    let mut doc = HEADER.to_owned();
    for (heading, rows) in sections {
        let rows_contexts: Vec<Contexts> = rows.iter().map(|row| row.contexts).collect();
        let section = section_contexts(&rows_contexts);
        write!(doc, "\n## {heading}\n\n").unwrap();
        if let Some(sentence) = contexts_text(section).0 {
            write!(doc, "{sentence}\n\n").unwrap();
        }
        doc.push_str("| Command | Default keys | Description |\n|---|---|---|\n");
        for row in rows {
            let Row {
                name,
                keys,
                description,
                contexts,
                windows_only,
            } = row;
            write!(doc, "| {name} | {keys} | {description}").unwrap();
            if contexts != section {
                write!(doc, " ({})", contexts_text(contexts).1).unwrap();
            }
            if windows_only {
                doc.push_str(" (Windows only)");
            }
            doc.push_str(" |\n");
        }
    }
    doc
}

fn jump_cells() -> (String, String) {
    let join = |cell: fn(char) -> String| {
        JUMP_CLASSES
            .iter()
            .map(|&(first, last)| format!("{} to {}", cell(first), cell(last)))
            .collect::<Vec<_>>()
            .join(", ")
    };
    (
        join(|c| code_span(&format!("jump_{c}"))),
        join(|c| code_span(&format!("Ctrl+{}", c.to_ascii_uppercase()))),
    )
}

// The contexts in which the configuration accepts a binding for the command:
// `nop` only unbinds a key, which the decoder allows in every context.
fn contexts_of(command: &Command) -> Contexts {
    CONTEXTS.map(|(context, _)| *command == Command::Nop || command.available_in(context))
}

// The most common contexts among the rows; on a tie, the file list alone if
// it is among the tied, otherwise those of the earliest row.
fn section_contexts(rows: &[Contexts]) -> Contexts {
    let count = |contexts: &Contexts| rows.iter().filter(|row| *row == contexts).count();
    match rows.iter().min_by_key(|row| Reverse(count(row))) {
        Some(common) if count(common) > count(&FILE_LIST_ONLY) => *common,
        _ => FILE_LIST_ONLY,
    }
}

// The sentence under a section's heading and the note after a row's
// description, for the contexts they work in.
fn contexts_text(contexts: Contexts) -> (Option<&'static str>, &'static str) {
    match contexts {
        [true, false, false] => (None, "file list only"),
        [true, true, false] => (
            Some("These commands also work in the image preview."),
            "file list and image preview",
        ),
        [true, false, true] => (
            Some("These commands also work in the text preview."),
            "file list and text preview",
        ),
        [true, true, true] => (
            Some("These commands work everywhere: in the file list and in both previews."),
            "everywhere",
        ),
        [false, true, false] => (
            Some("These commands work only in the image preview."),
            "image preview only",
        ),
        [false, false, true] => (
            Some("These commands work only in the text preview."),
            "text preview only",
        ),
        [false, true, true] => (
            Some("These commands work only in the previews."),
            "both previews",
        ),
        [false, false, false] => unreachable!("every command works in some context"),
    }
}

fn command_keys(keymap: &KeyMap, command: &Command) -> String {
    let contexts: Vec<(&str, Vec<String>)> = CONTEXTS
        .iter()
        .zip(contexts_of(command))
        .filter(|(_, works)| *works)
        .map(|((context, name), _)| {
            let keys = keymap.keys_for(*context, command);
            (*name, keys.iter().map(ToString::to_string).collect())
        })
        .collect();
    keys_cell(&contexts)
}

fn keys_cell(contexts: &[(&str, Vec<String>)]) -> String {
    let spans = |keys: &[String]| {
        keys.iter()
            .map(|key| code_span(key))
            .collect::<Vec<_>>()
            .join(", ")
    };
    match contexts {
        [(_, first), rest @ ..] if rest.iter().all(|(_, keys)| keys == first) => spans(first),
        _ => {
            let mut groups: Vec<(&[String], Vec<&str>)> = Vec::new();
            for (name, keys) in contexts.iter().filter(|(_, keys)| !keys.is_empty()) {
                match groups
                    .iter_mut()
                    .find(|(group, _)| *group == keys.as_slice())
                {
                    Some((_, names)) => names.push(name),
                    None => groups.push((keys, vec![name])),
                }
            }
            groups
                .iter()
                .map(|(keys, names)| format!("{} ({})", spans(keys), names.join(" and ")))
                .collect::<Vec<_>>()
                .join("; ")
        }
    }
}

// A key is a single character or a name with modifiers, so it holds at most
// one backtick, and a double-backtick span always holds it. The pipe is
// escaped even in a code span, since it would end the table cell.
fn code_span(text: &str) -> String {
    let text = text.replace('|', "\\|");
    if text.contains('`') {
        format!("`` {text} ``")
    } else {
        format!("`{text}`")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_start_at_the_first_command_and_follow_the_command_order() {
        let positions: Vec<Option<usize>> = SECTIONS
            .iter()
            .map(|(_, start)| Command::all().position(|command| command == *start))
            .collect();

        assert_eq!(positions[0], Some(0));
        assert!(
            positions.windows(2).all(|pair| pair[0] < pair[1]),
            "{positions:?}"
        );
    }

    #[test]
    fn every_jump_command_is_bound_to_ctrl_and_its_character() {
        let keymap = KeyMap::default_for(Os::Windows);
        let chars: Vec<char> = Command::all()
            .filter_map(|command| match command {
                Command::Jump(c) => Some(c),
                _ => None,
            })
            .collect();
        let classes: Vec<char> = JUMP_CLASSES
            .iter()
            .flat_map(|&(first, last)| first..=last)
            .collect();
        assert_eq!(chars, classes);

        for c in chars {
            let command = Command::Jump(c);
            for (context, _) in CONTEXTS {
                if command.available_in(context) {
                    let keys: Vec<String> = keymap
                        .keys_for(context, &command)
                        .iter()
                        .map(ToString::to_string)
                        .collect();
                    assert_eq!(
                        keys,
                        [format!("Ctrl+{}", c.to_ascii_uppercase())],
                        "{command} in {context:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn section_contexts_takes_the_most_common_contexts() {
        let both = [true, true, false];
        let previews = [false, true, true];

        assert_eq!(
            section_contexts(&[FILE_LIST_ONLY, both, both, FILE_LIST_ONLY, both]),
            both
        );
        assert_eq!(
            section_contexts(&[both, FILE_LIST_ONLY, previews, FILE_LIST_ONLY, both]),
            FILE_LIST_ONLY
        );
        assert_eq!(
            section_contexts(&[previews, both, both, previews]),
            previews
        );
    }

    #[test]
    fn keys_cell_writes_keys_shared_by_every_context_once() {
        let keys = vec!["Alt+Z".to_owned(), "Esc".to_owned()];

        assert_eq!(
            keys_cell(&[("file list", keys.clone()), ("text preview", keys)]),
            "`Alt+Z`, `Esc`"
        );
        assert_eq!(keys_cell(&[("file list", Vec::new())]), "");
    }

    #[test]
    fn keys_cell_names_the_contexts_when_they_differ() {
        assert_eq!(
            keys_cell(&[
                ("file list", vec!["Q".to_owned()]),
                ("image preview", Vec::new()),
                ("text preview", Vec::new()),
            ]),
            "`Q` (file list)"
        );
        assert_eq!(
            keys_cell(&[
                ("file list", vec!["Up".to_owned()]),
                ("image preview", vec!["Up".to_owned()]),
                ("text preview", vec!["E".to_owned(), "Enter".to_owned()]),
            ]),
            "`Up` (file list and image preview); `E`, `Enter` (text preview)"
        );
    }

    #[test]
    fn code_span_holds_backticks_and_escapes_pipes() {
        assert_eq!(code_span("`"), "`` ` ``");
        assert_eq!(code_span("Ctrl+`"), "`` Ctrl+` ``");
        assert_eq!(code_span("|"), "`\\|`");
        assert_eq!(code_span("\\"), "`\\`");
    }
}
