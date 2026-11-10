use std::borrow::Cow;
use std::ffi::OsString;
use std::iter::repeat_n;
use std::path::PathBuf;

use thiserror::Error;

use crate::app::Os;

/// A command line from the configuration: the program and its arguments,
/// split before the placeholders are filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandLine {
    program: String,
    args: Vec<Arg>,
}

/// What a placeholder argument stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placeholder {
    CursorPath,
    MarkedPaths,
    Paths,
    Dir,
    OtherDir,
}

/// The values of the placeholders for one invocation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Expansion {
    pub cursor_path: Option<PathBuf>,
    pub marked_paths: Vec<PathBuf>,
    pub paths: Vec<PathBuf>,
    pub dir: PathBuf,
    pub other_dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommandLineError {
    #[error("no program given")]
    Empty,
    #[error("unterminated quote or trailing backslash")]
    UnterminatedQuoteOrBackslash,
    #[error("unknown placeholder {0}")]
    UnknownPlaceholder(String),
}

impl CommandLine {
    /// Splits the text by the quoting rules of `os`: those of `sh` on macOS
    /// and Linux, without any expansion, and those of the C runtime on Windows.
    pub fn parse(text: &str, os: Os) -> Result<Self, CommandLineError> {
        let mut tokens = split(text, os)?;
        if tokens.is_empty() {
            return Err(CommandLineError::Empty);
        }
        let program = tokens.remove(0);
        let args = tokens
            .into_iter()
            .map(|token| match placeholder(&token) {
                Some(Ok(placeholder)) => Ok(Arg::Placeholder(placeholder)),
                Some(Err(error)) => Err(error),
                None => Ok(Arg::Literal(token)),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { program, args })
    }

    #[must_use]
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The arguments with each placeholder replaced by as many arguments as it
    /// has values, which may be none.
    #[must_use]
    pub fn expand(&self, expansion: &Expansion) -> Vec<OsString> {
        let mut args = Vec::new();
        for arg in &self.args {
            match arg {
                Arg::Literal(text) => args.push(OsString::from(text)),
                Arg::Placeholder(placeholder) => {
                    args.extend(expansion.values(*placeholder).map(OsString::from));
                }
            }
        }
        args
    }
}

impl Placeholder {
    fn from_name(name: &str) -> Option<Self> {
        Some(match name {
            "cursor_path" => Self::CursorPath,
            "marked_paths" => Self::MarkedPaths,
            "paths" => Self::Paths,
            "dir" => Self::Dir,
            "other_dir" => Self::OtherDir,
            _ => return None,
        })
    }
}

impl Expansion {
    fn values(&self, placeholder: Placeholder) -> impl Iterator<Item = &PathBuf> {
        let (single, many): (Option<&PathBuf>, &[PathBuf]) = match placeholder {
            Placeholder::CursorPath => (self.cursor_path.as_ref(), &[]),
            Placeholder::MarkedPaths => (None, &self.marked_paths),
            Placeholder::Paths => (None, &self.paths),
            Placeholder::Dir => (Some(&self.dir), &[]),
            Placeholder::OtherDir => (Some(&self.other_dir), &[]),
        };
        single.into_iter().chain(many)
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

#[derive(Debug, Clone, PartialEq, Eq)]
enum Arg {
    Literal(String),
    Placeholder(Placeholder),
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

fn quote_program(text: &str, os: Os) -> Cow<'_, str> {
    match os {
        // The program has no escapes, but one read by the program rule never
        // holds a `"`.
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

fn quote_arg(text: &str, os: Os) -> Cow<'_, str> {
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

// A placeholder is a whole argument; `{dir}` inside a longer argument is text.
fn placeholder(token: &str) -> Option<Result<Placeholder, CommandLineError>> {
    let name = token.strip_prefix('{')?.strip_suffix('}')?;
    Some(
        Placeholder::from_name(name)
            .ok_or_else(|| CommandLineError::UnknownPlaceholder(token.to_owned())),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_reads_the_program_and_the_arguments() {
        let line =
            CommandLine::parse(r#""C:\Program Files\app.exe"  -n  "a b"  "#, Os::Windows).unwrap();

        assert_eq!(line.program(), r"C:\Program Files\app.exe");
        assert_eq!(
            line.expand(&Expansion::default()),
            ["-n", "a b"].map(OsString::from)
        );
    }

    #[test]
    fn parse_rejects_empty_and_unterminated_text() {
        let cases = [
            (Os::Linux, "   ", CommandLineError::Empty),
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
    fn parse_recognizes_placeholders_only_as_whole_arguments() {
        let line = CommandLine::parse("code --dir={dir} {dir} \"{paths}\"", Os::Linux).unwrap();

        let expansion = Expansion {
            dir: PathBuf::from("/d"),
            paths: vec![PathBuf::from("/p1"), PathBuf::from("/p2")],
            ..Expansion::default()
        };
        assert_eq!(
            line.expand(&expansion),
            ["--dir={dir}", "/d", "/p1", "/p2"].map(OsString::from)
        );
    }

    #[test]
    fn parse_rejects_unknown_placeholders() {
        assert_eq!(
            CommandLine::parse("x {path}", Os::Linux),
            Err(CommandLineError::UnknownPlaceholder("{path}".to_owned()))
        );
    }

    #[test]
    fn expand_gives_each_placeholder_as_many_arguments_as_it_has_values() {
        let line = CommandLine::parse("x {cursor_path} {marked_paths} {other_dir} end", Os::Linux)
            .unwrap();

        let expansion = Expansion {
            cursor_path: None,
            marked_paths: vec![PathBuf::from("/m1"), PathBuf::from("/m2")],
            other_dir: PathBuf::from("/o"),
            ..Expansion::default()
        };
        assert_eq!(
            line.expand(&expansion),
            ["/m1", "/m2", "/o", "end"].map(OsString::from)
        );
    }

    #[test]
    fn format_line_quotes_by_the_rules_of_the_os() {
        let args = ["-a", "Wez Term", ""].map(OsString::from);

        assert_eq!(
            format_line(r"C:\My App\open", &args, Os::Windows),
            r#""C:\My App\open" -a "Wez Term" """#
        );
        assert_eq!(
            format_line("/My App/open", &args, Os::MacOs),
            "'/My App/open' -a 'Wez Term' ''"
        );
        assert_eq!(format_line("it's", &[], Os::Linux), r#""it's""#);
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
        for os in [Os::Windows, Os::MacOs, Os::Linux] {
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
            (" p a", &["", "p", "a"]),
            ("p\ta\tb", &["p", "a", "b"]),
            ("p a\nb\u{3000}c", &["p", "a\nb\u{3000}c"]),
        ];
        for (text, expected) in cases {
            assert_eq!(split(text, Os::Windows).unwrap(), expected, "{text:?}");
        }
    }

    #[test]
    fn split_on_unix_follows_sh_without_expansion() {
        let cases = [
            (r#"a 'b c' "d e" f\ g"#, &["a", "b c", "d e", "f g"][..]),
            ("echo $HOME ~ *", &["echo", "$HOME", "~", "*"]),
        ];
        for os in [Os::MacOs, Os::Linux] {
            for (text, expected) in cases {
                assert_eq!(split(text, os).unwrap(), expected, "{os:?} {text:?}");
            }
        }
    }
}
