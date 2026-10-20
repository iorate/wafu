use std::ffi::OsString;
use std::path::PathBuf;

use thiserror::Error;

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
    #[error("unterminated quote")]
    UnterminatedQuote,
    #[error("unknown placeholder {0}")]
    UnknownPlaceholder(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Arg {
    Literal(String),
    Placeholder(Placeholder),
}

impl CommandLine {
    /// Splits the text at spaces, keeping double-quoted parts together. A
    /// backslash is an ordinary character so that Windows paths survive.
    pub fn parse(text: &str) -> Result<Self, CommandLineError> {
        let mut tokens = split(text)?;
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

fn split(text: &str) -> Result<Vec<String>, CommandLineError> {
    let mut tokens = Vec::new();
    let mut current: Option<String> = None;
    let mut quoted = false;
    for c in text.chars() {
        match c {
            '"' => {
                quoted = !quoted;
                current.get_or_insert_with(String::new);
            }
            c if c.is_whitespace() && !quoted => {
                if let Some(token) = current.take() {
                    tokens.push(token);
                }
            }
            c => current.get_or_insert_with(String::new).push(c),
        }
    }
    if quoted {
        return Err(CommandLineError::UnterminatedQuote);
    }
    if let Some(token) = current {
        tokens.push(token);
    }
    Ok(tokens)
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
    fn parse_splits_at_spaces_and_keeps_quoted_parts_together() {
        let line =
            CommandLine::parse(r#"  "C:\Program Files\app.exe"  -n  "a b"  c  ""  "#).unwrap();

        assert_eq!(line.program(), r"C:\Program Files\app.exe");
        assert_eq!(
            line.expand(&Expansion::default()),
            ["-n", "a b", "c", ""].map(OsString::from)
        );
    }

    #[test]
    fn parse_treats_backslashes_as_text() {
        let line = CommandLine::parse(r"explorer C:\Users\me").unwrap();

        assert_eq!(
            line.expand(&Expansion::default()),
            [OsString::from(r"C:\Users\me")]
        );
    }

    #[test]
    fn parse_rejects_empty_and_unterminated_text() {
        assert_eq!(CommandLine::parse(""), Err(CommandLineError::Empty));
        assert_eq!(CommandLine::parse("   "), Err(CommandLineError::Empty));
        assert_eq!(
            CommandLine::parse(r#"a "b"#),
            Err(CommandLineError::UnterminatedQuote)
        );
    }

    #[test]
    fn parse_recognizes_placeholders_only_as_whole_arguments() {
        let line = CommandLine::parse("code --dir={dir} {dir} \"{paths}\"").unwrap();

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
            CommandLine::parse("x {path}"),
            Err(CommandLineError::UnknownPlaceholder("{path}".to_owned()))
        );
    }

    #[test]
    fn expand_gives_each_placeholder_as_many_arguments_as_it_has_values() {
        let line = CommandLine::parse("x {cursor_path} {marked_paths} {other_dir} end").unwrap();

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
}
