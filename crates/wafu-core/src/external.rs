use std::borrow::Cow;
use std::ffi::OsString;
use std::iter::repeat_n;
use std::path::PathBuf;

use thiserror::Error;

use crate::app::Os;
use crate::placeholder::{Piece, PlaceholderError, pieces};

/// A command line: the program and its arguments, split before the
/// placeholders are filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    source: String,
    program: String,
    args: Vec<Vec<Piece<Placeholder>>>,
}

/// What a placeholder stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder {
    CursorPath,
    MarkedPaths,
    Paths,
    Dir,
    OtherDir,
    Path,
}

/// The values of the placeholders for one invocation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Expansion {
    pub cursor_path: Option<PathBuf>,
    pub marked_paths: Vec<PathBuf>,
    pub paths: Vec<PathBuf>,
    pub dir: PathBuf,
    pub other_dir: PathBuf,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommandLineError {
    #[error("no program given")]
    Empty,
    #[error("unterminated quote or trailing backslash")]
    UnterminatedQuoteOrBackslash,
    #[error(transparent)]
    Placeholder(#[from] PlaceholderError),
    #[error("placeholder in the program")]
    PlaceholderInProgram,
    #[error("more than one list of paths in an argument")]
    ListsInOneArgument,
}

/// A placeholder that has no value, which keeps the line from being started.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
#[error("{{{}}} is empty", .0.name())]
pub struct EmptyPlaceholder(pub Placeholder);

impl CommandLine {
    /// Splits the text by the quoting rules of `os`: those of `sh` on macOS
    /// and Linux, without any expansion, and those of the C runtime on Windows.
    /// The placeholders are those of a line run on the active pane.
    pub fn parse(text: &str, os: Os) -> Result<Self, CommandLineError> {
        Self::parse_with(text, os, Placeholder::of_pane)
    }

    /// Like `parse`, for a line run on one file, whose only placeholder is
    /// `{path}`.
    pub fn parse_for_file(text: &str, os: Os) -> Result<Self, CommandLineError> {
        Self::parse_with(text, os, Placeholder::of_file)
    }

    /// The text the line was parsed from.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Whether an argument stands for paths of entries, which inside an
    /// archive have to be extracted first.
    #[must_use]
    pub fn uses_paths(&self) -> bool {
        self.args.iter().flatten().any(|piece| {
            matches!(
                piece,
                Piece::Placeholder(
                    Placeholder::CursorPath
                        | Placeholder::MarkedPaths
                        | Placeholder::Paths
                        | Placeholder::Path
                )
            )
        })
    }

    /// The arguments with the placeholders filled in. An argument that holds
    /// a list of paths is repeated once for each of them.
    pub fn expand(&self, expansion: &Expansion) -> Result<Vec<OsString>, EmptyPlaceholder> {
        let mut args = Vec::new();
        for arg in &self.args {
            let mut expanded = vec![OsString::new()];
            for piece in arg {
                match piece {
                    Piece::Text(text) => {
                        for arg in &mut expanded {
                            arg.push(text);
                        }
                    }
                    Piece::Placeholder(placeholder) => {
                        let values = expansion.values(*placeholder);
                        if values.is_empty() {
                            return Err(EmptyPlaceholder(*placeholder));
                        }
                        expanded = expanded
                            .iter()
                            .flat_map(|arg| {
                                values.iter().map(move |value| {
                                    let mut arg = arg.clone();
                                    arg.push(value);
                                    arg
                                })
                            })
                            .collect();
                    }
                }
            }
            args.extend(expanded);
        }
        Ok(args)
    }

    fn parse_with(
        text: &str,
        os: Os,
        placeholder: fn(&str) -> Option<Placeholder>,
    ) -> Result<Self, CommandLineError> {
        let mut tokens = split(text, os)?;
        if tokens.first().is_none_or(String::is_empty) {
            return Err(CommandLineError::Empty);
        }
        let program = match pieces(&tokens.remove(0), placeholder)?.as_slice() {
            [Piece::Text(text)] => text.clone(),
            _ => return Err(CommandLineError::PlaceholderInProgram),
        };
        let args = tokens
            .iter()
            .map(|token| {
                let pieces = pieces(token, placeholder)?;
                let lists = pieces
                    .iter()
                    .filter(|piece| matches!(piece, Piece::Placeholder(p) if p.is_list()))
                    .count();
                if lists > 1 {
                    return Err(CommandLineError::ListsInOneArgument);
                }
                Ok(pieces)
            })
            .collect::<Result<_, _>>()?;
        Ok(Self {
            source: text.to_owned(),
            program,
            args,
        })
    }
}

impl Placeholder {
    fn of_file(name: &str) -> Option<Self> {
        (name == "path").then_some(Self::Path)
    }

    fn of_pane(name: &str) -> Option<Self> {
        Some(match name {
            "cursor_path" => Self::CursorPath,
            "marked_paths" => Self::MarkedPaths,
            "paths" => Self::Paths,
            "dir" => Self::Dir,
            "other_dir" => Self::OtherDir,
            _ => return None,
        })
    }

    fn name(self) -> &'static str {
        match self {
            Self::CursorPath => "cursor_path",
            Self::MarkedPaths => "marked_paths",
            Self::Paths => "paths",
            Self::Dir => "dir",
            Self::OtherDir => "other_dir",
            Self::Path => "path",
        }
    }

    fn is_list(self) -> bool {
        matches!(self, Self::MarkedPaths | Self::Paths)
    }
}

impl Expansion {
    fn values(&self, placeholder: Placeholder) -> &[PathBuf] {
        match placeholder {
            Placeholder::CursorPath => self.cursor_path.as_slice(),
            Placeholder::MarkedPaths => &self.marked_paths,
            Placeholder::Paths => &self.paths,
            Placeholder::Dir => dir_values(&self.dir),
            Placeholder::OtherDir => dir_values(&self.other_dir),
            Placeholder::Path => self.path.as_slice(),
        }
    }
}

/// The program and its arguments as one line, quoted so that the rules of
/// the same `os` split it back into the same parts.
#[must_use]
pub fn format_line(program: &str, args: &[OsString], os: Os) -> String {
    let args = args
        .iter()
        .map(|arg| quote_arg(&arg.to_string_lossy(), os).into_owned());
    std::iter::once(quote_program(program, os).into_owned())
        .chain(args)
        .collect::<Vec<_>>()
        .join(" ")
}

pub(crate) fn escape_braces(text: &str) -> String {
    text.replace('{', "{{").replace('}', "}}")
}

// The rules follow the `Os` value rather than `cfg` so that both sets are
// compiled and tested on every host.
fn split(text: &str, os: Os) -> Result<Vec<String>, CommandLineError> {
    match os {
        Os::Windows => Ok(split_windows(text)),
        Os::MacOs | Os::Linux => {
            shlex::split(text).ok_or(CommandLineError::UnterminatedQuoteOrBackslash)
        }
    }
}

// Hand-written because no maintained crate both splits and quotes by the
// current C runtime rules on every host.
fn split_windows(text: &str) -> Vec<String> {
    let mut chars = text.chars().peekable();
    let mut program = String::new();
    let mut quoted = false;
    for c in chars.by_ref() {
        match c {
            '"' => quoted = !quoted,
            ' ' | '\t' if !quoted => break,
            c => program.push(c),
        }
    }
    let mut tokens = vec![program];
    let mut current: Option<String> = None;
    let mut quoted = false;
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' if !quoted => {
                if let Some(token) = current.take() {
                    tokens.push(token);
                }
            }
            '\\' => {
                let mut count = 1;
                while chars.next_if_eq(&'\\').is_some() {
                    count += 1;
                }
                let token = current.get_or_insert_with(String::new);
                if chars.peek() == Some(&'"') {
                    token.extend(repeat_n('\\', count / 2));
                    if count % 2 == 1 {
                        chars.next();
                        token.push('"');
                    }
                } else {
                    token.extend(repeat_n('\\', count));
                }
            }
            '"' => {
                let token = current.get_or_insert_with(String::new);
                if quoted && chars.next_if_eq(&'"').is_some() {
                    token.push('"');
                } else {
                    quoted = !quoted;
                }
            }
            c => current.get_or_insert_with(String::new).push(c),
        }
    }
    if let Some(token) = current {
        tokens.push(token);
    }
    tokens
}

// A pane without a location has an empty path for its directory.
fn dir_values(dir: &PathBuf) -> &[PathBuf] {
    if dir.as_os_str().is_empty() {
        &[]
    } else {
        std::slice::from_ref(dir)
    }
}

pub(crate) fn quote_program(text: &str, os: Os) -> Cow<'_, str> {
    match os {
        // The program rule has no escapes, so a `"` could not be written, but a
        // program it read never holds one.
        Os::Windows => {
            if text.contains([' ', '\t']) {
                Cow::Owned(format!("\"{text}\""))
            } else {
                Cow::Borrowed(text)
            }
        }
        Os::MacOs | Os::Linux => quote_arg(text, os),
    }
}

pub(crate) fn quote_arg(text: &str, os: Os) -> Cow<'_, str> {
    match os {
        Os::Windows => quote_windows_arg(text),
        // `try_quote` fails only on a NUL, which spawning rejects, so no line
        // that is shown can hold one.
        Os::MacOs | Os::Linux => shlex::try_quote(text).unwrap_or(Cow::Borrowed(text)),
    }
}

// The inverse of the argument rules of `split_windows`, as the Rust standard
// library quotes arguments for `Command` on Windows.
fn quote_windows_arg(text: &str) -> Cow<'_, str> {
    let quoted = text.is_empty() || text.contains([' ', '\t']);
    if !quoted && !text.contains('"') {
        return Cow::Borrowed(text);
    }
    let mut result = String::new();
    if quoted {
        result.push('"');
    }
    let mut backslashes = 0;
    for c in text.chars() {
        if c == '\\' {
            backslashes += 1;
        } else {
            if c == '"' {
                result.extend(repeat_n('\\', backslashes + 1));
            }
            backslashes = 0;
        }
        result.push(c);
    }
    if quoted {
        result.extend(repeat_n('\\', backslashes));
        result.push('"');
    }
    Cow::Owned(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_the_program_and_the_arguments() {
        let line = CommandLine::parse("p -n a", Os::Linux).unwrap();

        assert_eq!(line.program(), "p");
        assert_eq!(
            line.expand(&Expansion::default()).unwrap(),
            ["-n", "a"].map(OsString::from)
        );
    }

    #[test]
    fn parse_rejects_an_empty_program_and_unterminated_text() {
        let cases = [
            (Os::Linux, "   ", CommandLineError::Empty),
            (Os::Linux, r#""" a"#, CommandLineError::Empty),
            (
                Os::Linux,
                r#"a "b"#,
                CommandLineError::UnterminatedQuoteOrBackslash,
            ),
        ];
        for (os, text, error) in cases {
            assert_eq!(CommandLine::parse(text, os), Err(error), "{os:?} {text:?}");
        }
    }

    #[test]
    fn parse_rejects_bad_placeholders() {
        let cases = [
            ("{dir} x", CommandLineError::PlaceholderInProgram),
            (
                "x {marked_paths}:{paths}",
                CommandLineError::ListsInOneArgument,
            ),
        ];
        for (text, error) in cases {
            assert_eq!(CommandLine::parse(text, Os::Linux), Err(error), "{text:?}");
        }
    }

    #[test]
    fn parse_takes_the_placeholders_of_its_own_kind_of_line_only() {
        let unknown = |name: &str| {
            Err(CommandLineError::Placeholder(PlaceholderError::Unknown(
                name.to_owned(),
            )))
        };

        assert_eq!(CommandLine::parse("x {path}", Os::Linux), unknown("path"));
        for name in ["cursor_path", "marked_paths", "paths", "dir", "other_dir"] {
            let text = format!("x {{{name}}}");

            assert_eq!(CommandLine::parse_for_file(&text, Os::Linux), unknown(name));
        }
    }

    #[test]
    fn parse_reads_doubled_braces_in_the_program_as_text() {
        let line = CommandLine::parse("{{p}}", Os::Linux).unwrap();

        assert_eq!(line.program(), "{p}");
    }

    #[test]
    fn uses_paths_is_true_only_for_a_placeholder_of_entry_paths() {
        let cases = [
            ("x {cursor_path}", true),
            ("x {marked_paths}", true),
            ("x {paths}", true),
            ("x {dir} {other_dir}", false),
        ];
        for (text, expected) in cases {
            let line = CommandLine::parse(text, Os::Linux).unwrap();

            assert_eq!(line.uses_paths(), expected, "{text:?}");
        }
    }

    #[test]
    fn uses_paths_is_true_for_the_placeholder_of_a_file() {
        let line = CommandLine::parse_for_file("x {path}", Os::Linux).unwrap();

        assert!(line.uses_paths());
    }

    #[test]
    fn expand_fills_in_placeholders_anywhere_in_an_argument() {
        let line = CommandLine::parse(
            "x {dir} --dir={dir} \"{cursor_path}\" {dir}:{other_dir} {paths}",
            Os::Linux,
        )
        .unwrap();

        let expansion = Expansion {
            cursor_path: Some(PathBuf::from("/c d")),
            paths: vec![PathBuf::from("/p1"), PathBuf::from("/p2")],
            dir: PathBuf::from("/d"),
            other_dir: PathBuf::from("/o"),
            ..Expansion::default()
        };
        assert_eq!(
            line.expand(&expansion).unwrap(),
            ["/d", "--dir=/d", "/c d", "/d:/o", "/p1", "/p2"].map(OsString::from)
        );
    }

    #[test]
    fn expand_repeats_an_argument_for_each_path_of_its_list() {
        let line = CommandLine::parse("x {dir}/{marked_paths}.bak", Os::Linux).unwrap();

        let expansion = Expansion {
            marked_paths: vec![PathBuf::from("m1"), PathBuf::from("m2")],
            dir: PathBuf::from("/d"),
            ..Expansion::default()
        };
        assert_eq!(
            line.expand(&expansion).unwrap(),
            ["/d/m1.bak", "/d/m2.bak"].map(OsString::from)
        );
    }

    #[test]
    fn expand_fails_on_a_placeholder_without_a_value() {
        type Parse = fn(&str, Os) -> Result<CommandLine, CommandLineError>;

        let full = Expansion {
            cursor_path: Some(PathBuf::from("/c")),
            marked_paths: vec![PathBuf::from("/m")],
            paths: vec![PathBuf::from("/p")],
            dir: PathBuf::from("/d"),
            other_dir: PathBuf::from("/o"),
            path: Some(PathBuf::from("/f")),
        };
        let pane: Parse = CommandLine::parse;
        let file: Parse = CommandLine::parse_for_file;
        let cases = [
            (
                pane,
                "x {cursor_path}",
                Expansion {
                    cursor_path: None,
                    ..full.clone()
                },
                "{cursor_path} is empty",
            ),
            (
                pane,
                "x {marked_paths}",
                Expansion {
                    marked_paths: Vec::new(),
                    ..full.clone()
                },
                "{marked_paths} is empty",
            ),
            (
                pane,
                "x {paths}",
                Expansion {
                    paths: Vec::new(),
                    ..full.clone()
                },
                "{paths} is empty",
            ),
            (
                pane,
                "x {dir}",
                Expansion {
                    dir: PathBuf::new(),
                    ..full.clone()
                },
                "{dir} is empty",
            ),
            (
                pane,
                "x {other_dir}",
                Expansion {
                    other_dir: PathBuf::new(),
                    ..full.clone()
                },
                "{other_dir} is empty",
            ),
            (
                file,
                "x {path}",
                Expansion {
                    path: None,
                    ..full.clone()
                },
                "{path} is empty",
            ),
        ];
        for (parse, text, expansion, message) in cases {
            let line = parse(text, Os::Linux).unwrap();

            let error = line.expand(&expansion).unwrap_err();
            assert_eq!(error.to_string(), message, "{text:?}");
        }
    }

    #[test]
    fn escape_braces_is_read_back_as_the_same_text() {
        let text = "{dir} }{ {{a}}";

        let line = CommandLine::parse(&format!("p {}", escape_braces(text)), Os::Windows).unwrap();

        assert_eq!(
            line.expand(&Expansion::default()).unwrap(),
            ["{dir}", "}{", "{{a}}"].map(OsString::from)
        );
    }

    #[test]
    fn format_line_quotes_only_the_parts_that_need_it_on_windows() {
        let args = ["-a", "Wez Term", ""].map(OsString::from);

        assert_eq!(
            format_line(r"C:\My App\open", &args, Os::Windows),
            r#""C:\My App\open" -a "Wez Term" """#
        );
    }

    #[test]
    fn format_line_is_read_back_as_the_same_arguments() {
        let args = [
            "a b",
            "a\tb",
            "",
            r#"say "hi""#,
            "it's",
            r"C:\dir\",
            r"C:\my dir\",
            r#"\\"\\"#,
        ];
        let program = r"C:\my dir\";
        let mut expected = vec![program];
        expected.extend(args);
        for os in [Os::Windows, Os::Linux] {
            let line = format_line(program, &args.map(OsString::from), os);

            assert_eq!(split(&line, os).unwrap(), expected, "{os:?} {line:?}");
        }
    }

    #[test]
    fn split_on_windows_follows_the_c_runtime() {
        let cases = [
            // The first five rows are Microsoft's examples, after a program since
            // they show argv[1..].
            (r#"p "abc" d e"#, &["p", "abc", "d", "e"][..]),
            (r#"p a\\b d"e f"g h"#, &["p", r"a\\b", "de fg", "h"]),
            (r#"p a\\\"b c d"#, &["p", r#"a\"b"#, "c", "d"]),
            (r#"p a\\\\"b c" d e"#, &["p", r"a\\b c", "d", "e"]),
            (r#"p a"b"" c d"#, &["p", r#"ab" c d"#]),
            (r#"C:\dir\"a b"\x y"#, &[r"C:\dir\a b\x", "y"]),
            (r#""a""b" c"#, &["ab", "c"]),
            (r#"p "" "#, &["p", ""]),
            (r#"p "a b"#, &["p", "a b"]),
            (" p a", &["", "p", "a"]),
            ("p\ta\tb", &["p", "a", "b"]),
            ("p a\nb\u{3000}c", &["p", "a\nb\u{3000}c"]),
        ];
        for (text, expected) in cases {
            assert_eq!(split(text, Os::Windows).unwrap(), expected, "{text:?}");
        }
    }
}
