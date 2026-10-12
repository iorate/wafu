use std::ffi::OsStr;
use std::fmt;

use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use thiserror::Error;

/// A set of case-insensitive globs separated by `;`, applied to file names.
#[derive(Debug, Clone)]
pub struct Filter {
    pattern: String,
    set: GlobSet,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct FilterError(String);

impl Filter {
    /// Parses the text of the input field. Returns `None` when no pattern remains,
    /// which clears the filter.
    pub fn parse(text: &str) -> Result<Option<Self>, FilterError> {
        let mut builder = GlobSetBuilder::new();
        let mut any = false;
        for part in text
            .split(';')
            .map(str::trim)
            .filter(|part| !part.is_empty())
        {
            let glob = GlobBuilder::new(part)
                .case_insensitive(true)
                .literal_separator(false)
                .build()
                .map_err(|error| FilterError(error.kind().to_string()))?;
            builder.add(glob);
            any = true;
        }
        if !any {
            return Ok(None);
        }
        let set = builder
            .build()
            .map_err(|error| FilterError(error.kind().to_string()))?;
        Ok(Some(Self {
            pattern: text.trim().to_owned(),
            set,
        }))
    }

    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    #[must_use]
    pub fn matches(&self, name: &OsStr) -> bool {
        self.set.is_match(name.to_string_lossy().as_ref())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_matches_any_of_the_patterns() {
        let filter = Filter::parse("*.png;*.jpg").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("a.png")));
        assert!(filter.matches(OsStr::new("b.jpg")));
        assert!(!filter.matches(OsStr::new("c.txt")));
        assert_eq!(filter.pattern(), "*.png;*.jpg");
    }

    #[test]
    fn parse_ignores_case() {
        let filter = Filter::parse("*.PNG").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("a.png")));
        assert!(filter.matches(OsStr::new("A.Png")));
    }

    #[test]
    fn parse_skips_empty_parts_and_spaces() {
        let filter = Filter::parse(" *.png ; ; ").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("a.png")));
        assert!(!filter.matches(OsStr::new("a.txt")));
    }

    #[test]
    fn parse_of_empty_text_clears_the_filter() {
        assert_eq!(Filter::parse("").unwrap(), None);
        assert_eq!(Filter::parse(" ; ;").unwrap(), None);
    }

    #[test]
    fn parse_rejects_invalid_globs() {
        assert!(Filter::parse("[").is_err());
        assert!(Filter::parse("*.png;[").is_err());
    }

    #[test]
    fn equality_is_by_pattern() {
        let a = Filter::parse("*.png").unwrap().unwrap();
        let b = Filter::parse("*.png").unwrap().unwrap();
        let c = Filter::parse("*.jpg").unwrap().unwrap();

        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn star_matches_across_dots() {
        let filter = Filter::parse("a*").unwrap().unwrap();

        assert!(filter.matches(OsStr::new("a.tar.gz")));
        assert!(!filter.matches(OsStr::new("ba")));
    }
}
