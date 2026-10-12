use std::fmt;
use std::path::PathBuf;

/// A place shown in a pane: a real path, optionally followed by a path inside an archive.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    pub base: PathBuf,
    pub archive: Option<ArchivePath>,
}

/// A directory path inside an archive. Empty means the archive root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArchivePath {
    pub inner: Vec<String>,
}

impl Location {
    #[must_use]
    pub fn new(base: PathBuf) -> Self {
        Self {
            base,
            archive: None,
        }
    }

    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        match &self.archive {
            None => self
                .base
                .parent()
                .map(|parent| Self::new(parent.to_path_buf())),
            Some(_) => None,
        }
    }
}

impl fmt::Display for Location {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.base.display())?;
        if let Some(archive) = &self.archive {
            for segment in &archive.inner {
                write!(f, "/{segment}")?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parent_of_root_is_none() {
        let root = Location::new(PathBuf::from("/"));

        assert_eq!(root.parent(), None);
    }

    #[test]
    fn parent_strips_last_component() {
        let location = Location::new(PathBuf::from("/home/user"));

        assert_eq!(
            location.parent(),
            Some(Location::new(PathBuf::from("/home")))
        );
    }

    #[test]
    fn display_appends_archive_path() {
        let location = Location {
            base: PathBuf::from("/a/b/c.zip"),
            archive: Some(ArchivePath {
                inner: vec!["d".to_owned(), "e".to_owned()],
            }),
        };

        assert_eq!(location.to_string(), "/a/b/c.zip/d/e");
    }
}
