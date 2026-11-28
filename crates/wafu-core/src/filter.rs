use std::ffi::OsStr;
use std::fmt;
use std::path::Path;

use globset::{GlobBuilder, GlobMatcher};
use thiserror::Error;

use crate::text::nfc;

/// A case-insensitive glob applied to file names. Alternatives are written
/// with braces, as in `*.{png,jpg}`. Globs after `~` exclude names, as in
/// `*~*.bak`; a literal `~` is written `\~` or `[~]`.
///
/// A glob may also be applied to a path below a directory, by
/// [`Filter::matches_path`], with the slash rules of gitignore: a glob
/// without a slash matches a name at any depth, one with a slash matches the
/// whole path, a leading slash anchors the glob to the directory searched,
/// and a trailing slash matches directories only.
#[derive(Debug, Clone)]
pub struct Filter {
    pattern: String,
    include: Glob,
    excludes: Vec<Glob>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct FilterError(String);

impl Filter {
    /// Parses the text of the input field. Returns `None` when it is empty,
    /// which clears the filter.
    pub fn parse(text: &str) -> Result<Option<Self>, FilterError> {
        let pattern = nfc(text);
        if pattern.is_empty() {
            return Ok(None);
        }
        let parts = split_parts(&pattern);
        if parts.iter().any(|part| part.is_empty()) {
            return Err(FilterError("empty glob next to '~'".to_owned()));
        }
        let include = Glob::new(parts[0])?;
        let excludes = parts[1..]
            .iter()
            .map(|part| Glob::new(part))
            .collect::<Result<_, _>>()?;
        Ok(Some(Self {
            pattern: pattern.into_owned(),
            include,
            excludes,
        }))
    }

    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    #[must_use]
    pub fn matches(&self, name: &OsStr) -> bool {
        let name = name.to_string_lossy();
        let name = nfc(&name);
        self.include.name.is_match(name.as_ref())
            && !self
                .excludes
                .iter()
                .any(|exclude| exclude.name.is_match(name.as_ref()))
    }

    /// Whether an entry matches by its path relative to the directory
    /// searched.
    #[must_use]
    pub fn matches_path(&self, relative: &Path, is_dir: bool) -> bool {
        let path = relative.to_string_lossy();
        let path = nfc(&path);
        let matches = |glob: &Glob| (is_dir || !glob.dir_only) && glob.path.is_match(path.as_ref());
        matches(&self.include) && !self.excludes.iter().any(matches)
    }
}

impl PartialEq for Filter {
    fn eq(&self, other: &Self) -> bool {
        self.pattern == other.pattern
    }
}

impl Eq for Filter {}

impl fmt::Display for Filter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.pattern)
    }
}

#[derive(Debug, Clone)]
struct Glob {
    name: GlobMatcher,
    path: GlobMatcher,
    dir_only: bool,
}

impl Glob {
    fn new(glob: &str) -> Result<Self, FilterError> {
        let (path, anchored) = match glob.strip_prefix('/') {
            Some(rest) => (rest, true),
            None => (glob, false),
        };
        let (path, dir_only) = match path.strip_suffix('/') {
            Some(rest) => (rest, true),
            None => (path, false),
        };
        let path = if anchored || path.contains('/') {
            path.to_owned()
        } else {
            format!("**/{path}")
        };
        Ok(Self {
            name: matcher(glob, false)?,
            path: matcher(&path, true)?,
            dir_only,
        })
    }
}

fn matcher(glob: &str, literal_separator: bool) -> Result<GlobMatcher, FilterError> {
    GlobBuilder::new(glob)
        .case_insensitive(true)
        .literal_separator(literal_separator)
        .backslash_escape(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|error| FilterError(error.kind().to_string()))
}

fn split_parts(pattern: &str) -> Vec<&str> {
    let chars: Vec<(usize, char)> = pattern.char_indices().collect();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth = 0usize;
    let mut i = 0;
    while i < chars.len() {
        match chars[i].1 {
            '\\' => i += 1,
            '[' => {
                // Without a closing `]`, globset reports the unclosed class.
                if let Some(end) = class_end(&chars, i) {
                    i = end;
                }
            }
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            '~' if depth == 0 => {
                parts.push(&pattern[start..chars[i].0]);
                start = chars[i].0 + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&pattern[start..]);
    parts
}

// Follows globset: a `]` right after `[`, `[!` or `[^` is a member, and `\`
// is not an escape inside a class.
fn class_end(chars: &[(usize, char)], open: usize) -> Option<usize> {
    let mut i = open + 1;
    if matches!(chars.get(i), Some((_, '!' | '^'))) {
        i += 1;
    }
    if matches!(chars.get(i), Some((_, ']'))) {
        i += 1;
    }
    chars[i..]
        .iter()
        .position(|&(_, c)| c == ']')
        .map(|offset| i + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_matches_alternatives_in_braces() {
        let filter = Filter::parse("*.{png,jpg}").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("a.png")));
        assert!(filter.matches(OsStr::new("b.jpg")));
        assert!(!filter.matches(OsStr::new("c.txt")));
        assert_eq!(filter.pattern(), "*.{png,jpg}");
    }

    #[test]
    fn parse_excludes_names_matching_a_glob_after_a_tilde() {
        let filter = Filter::parse("*~b*~c*").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("a")));
        assert!(!filter.matches(OsStr::new("b1")));
        assert!(!filter.matches(OsStr::new("c1")));
        assert_eq!(filter.pattern(), "*~b*~c*");
    }

    #[test]
    fn parse_does_not_split_at_an_escaped_tilde() {
        let filter = Filter::parse(r"*~*\~").unwrap().unwrap();
        assert!(filter.matches(OsStr::new("a")));
        assert!(!filter.matches(OsStr::new("a~")));

        let filter = Filter::parse(r"*~\~$*").unwrap().unwrap();
        assert!(filter.matches(OsStr::new("a")));
        assert!(!filter.matches(OsStr::new("~$a")));
    }

    #[test]
    fn parse_does_not_split_at_a_tilde_in_a_class() {
        let filter = Filter::parse("PROGRA[~]1").unwrap().unwrap();
        assert!(filter.matches(OsStr::new("PROGRA~1")));

        let filter = Filter::parse("[]~]*~*x").unwrap().unwrap();
        assert!(filter.matches(OsStr::new("~a")));
        assert!(filter.matches(OsStr::new("]a")));
        assert!(!filter.matches(OsStr::new("~x")));

        let filter = Filter::parse("[!~]*~a*").unwrap().unwrap();
        assert!(filter.matches(OsStr::new("b")));
        assert!(!filter.matches(OsStr::new("~b")));
        assert!(!filter.matches(OsStr::new("ab")));

        let filter = Filter::parse("[!]~]*~*x").unwrap().unwrap();
        assert!(filter.matches(OsStr::new("a")));
        assert!(!filter.matches(OsStr::new("~a")));
        assert!(!filter.matches(OsStr::new("ax")));

        let filter = Filter::parse("*.[ch~]~*.h").unwrap().unwrap();
        assert!(filter.matches(OsStr::new("a.c")));
        assert!(!filter.matches(OsStr::new("a.h")));
    }

    #[test]
    fn parse_does_not_split_at_a_tilde_in_braces() {
        let filter = Filter::parse("*.{txt~,bak}").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("a.txt~")));
        assert!(filter.matches(OsStr::new("a.bak")));
        assert!(!filter.matches(OsStr::new("a.txt")));
    }

    #[test]
    fn parse_splits_at_a_tilde_after_braces() {
        let filter = Filter::parse("*.{c,h}~a*").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("b.c")));
        assert!(!filter.matches(OsStr::new("a.c")));
    }

    #[test]
    fn parse_keeps_spaces() {
        let filter = Filter::parse("* ~ ~$*").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("a ")));
        assert!(!filter.matches(OsStr::new("a")));
        assert!(!filter.matches(OsStr::new(" ")));
        assert!(!filter.matches(OsStr::new("$a ")));
        assert_eq!(filter.pattern(), "* ~ ~$*");
    }

    #[test]
    fn parse_of_empty_text_clears_the_filter() {
        assert_eq!(Filter::parse("").unwrap(), None);

        let filter = Filter::parse("   ").unwrap().unwrap();
        assert!(filter.matches(OsStr::new("   ")));
        assert!(!filter.matches(OsStr::new("a")));
    }

    #[test]
    fn parse_rejects_an_empty_part() {
        for text in ["*~", "~$*", "*~~x", "~"] {
            assert_eq!(
                Filter::parse(text).unwrap_err(),
                FilterError("empty glob next to '~'".to_owned()),
                "{text}"
            );
        }
    }

    #[test]
    fn parse_rejects_an_invalid_pattern() {
        assert!(Filter::parse("[").is_err());
        assert!(Filter::parse("*~{").is_err());
    }

    #[test]
    fn matches_path_matches_a_glob_without_a_slash_at_any_depth() {
        let filter = Filter::parse("*.rs").unwrap().unwrap();

        assert!(filter.matches_path(Path::new("a.rs"), false));
        assert!(filter.matches_path(&Path::new("src").join("b").join("a.rs"), false));
        assert!(!filter.matches_path(&Path::new("a.rs").join("b"), false));
    }

    #[test]
    fn matches_path_matches_a_glob_with_a_slash_against_the_whole_path() {
        let filter = Filter::parse("src/*.rs").unwrap().unwrap();

        assert!(filter.matches_path(&Path::new("src").join("a.rs"), false));
        assert!(!filter.matches_path(&Path::new("src").join("b").join("a.rs"), false));
        assert!(!filter.matches_path(&Path::new("x").join("src").join("a.rs"), false));
    }

    #[test]
    fn matches_path_anchors_a_glob_with_a_leading_slash() {
        let filter = Filter::parse("/a.rs").unwrap().unwrap();

        assert!(filter.matches_path(Path::new("a.rs"), false));
        assert!(!filter.matches_path(&Path::new("src").join("a.rs"), false));
    }

    #[test]
    fn matches_path_matches_nothing_with_a_lone_slash() {
        let filter = Filter::parse("/").unwrap().unwrap();

        assert!(!filter.matches_path(Path::new("src"), true));
        assert!(!filter.matches_path(Path::new("a.rs"), false));
    }

    #[test]
    fn matches_path_matches_only_directories_with_a_trailing_slash() {
        let filter = Filter::parse("src/").unwrap().unwrap();

        assert!(filter.matches_path(Path::new("src"), true));
        assert!(filter.matches_path(&Path::new("x").join("src"), true));
        assert!(!filter.matches_path(Path::new("src"), false));
    }

    #[test]
    fn matches_path_excludes_by_the_same_rules_entry_by_entry() {
        let filter = Filter::parse("*~/gen/*~target/").unwrap().unwrap();

        assert!(!filter.matches_path(&Path::new("gen").join("a.rs"), false));
        assert!(filter.matches_path(&Path::new("x").join("gen").join("a.rs"), false));
        assert!(!filter.matches_path(Path::new("target"), true));
        assert!(filter.matches_path(Path::new("target"), false));
        assert!(filter.matches_path(&Path::new("target").join("a.rs"), false));
    }

    #[test]
    fn matches_path_ignores_case_and_composes_decomposed_names() {
        let filter = Filter::parse("src/*\u{30d7}*").unwrap().unwrap();

        assert!(filter.matches_path(&Path::new("SRC").join("\u{30d5}\u{309a}"), false));
    }

    #[test]
    fn matches_does_not_match_a_glob_with_a_slash() {
        let filter = Filter::parse("src/").unwrap().unwrap();

        assert!(!filter.matches(OsStr::new("src")));
    }

    #[test]
    fn matching_ignores_case() {
        let filter = Filter::parse("*.png~b*").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("c.PNG")));
        assert!(!filter.matches(OsStr::new("B.png")));
    }

    #[test]
    fn matching_composes_decomposed_names() {
        let filter = Filter::parse("*\u{30d7}*").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("\u{30a2}\u{30d5}\u{309a}\u{30ea}")));
    }

    #[test]
    fn star_matches_across_dots() {
        let filter = Filter::parse("a*").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("a.tar.gz")));
        assert!(!filter.matches(OsStr::new("ba")));
    }
}
