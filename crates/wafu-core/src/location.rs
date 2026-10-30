use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};

/// A place shown in a pane: a real path, optionally followed by a path inside an archive.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Location {
    pub base: PathBuf,
    pub archive: Option<ArchivePath>,
}

/// A directory inside an archive. An empty `inner` means the archive root.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArchivePath {
    pub format: ArchiveFormat,
    pub inner: Vec<String>,
}

/// An archive format the I/O layer can read and write. Adding one means
/// adding a variant here, its extension below, and an implementation in the
/// I/O layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveFormat {
    Zip,
}

impl ArchiveFormat {
    /// The format a file name stands for, by its extension. Case does not matter.
    #[must_use]
    pub fn from_name(name: &OsStr) -> Option<Self> {
        let extension = Path::new(name).extension()?;
        if extension.eq_ignore_ascii_case("zip") {
            Some(Self::Zip)
        } else {
            None
        }
    }

    /// The extension given to a new archive, without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
        }
    }
}

impl Location {
    #[must_use]
    pub fn new(base: PathBuf) -> Self {
        Self {
            base,
            archive: None,
        }
    }

    /// The root of the archive at `base`.
    #[must_use]
    pub fn archive_root(base: PathBuf, format: ArchiveFormat) -> Self {
        Self {
            base,
            archive: Some(ArchivePath {
                format,
                inner: Vec::new(),
            }),
        }
    }

    #[must_use]
    pub fn in_archive(&self) -> bool {
        self.archive.is_some()
    }

    /// Whether `parent` returns a place, without building it.
    #[must_use]
    pub fn has_parent(&self) -> bool {
        match &self.archive {
            Some(archive) if !archive.inner.is_empty() => true,
            _ => self.base.parent().is_some(),
        }
    }

    /// The place above this one: the directory above a directory, the
    /// directory holding an archive above its root.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        match &self.archive {
            Some(archive) if !archive.inner.is_empty() => {
                let mut inner = archive.inner.clone();
                inner.pop();
                Some(Self {
                    base: self.base.clone(),
                    archive: Some(ArchivePath {
                        format: archive.format,
                        inner,
                    }),
                })
            }
            _ => self
                .base
                .parent()
                .map(|parent| Self::new(parent.to_path_buf())),
        }
    }

    /// The name this place has in its parent, which is where the cursor goes
    /// after moving up.
    #[must_use]
    pub fn leaf_name(&self) -> Option<OsString> {
        match &self.archive {
            Some(archive) if !archive.inner.is_empty() => archive.inner.last().map(OsString::from),
            _ => self.base.file_name().map(OsString::from),
        }
    }

    /// A directory inside the archive below this place.
    #[must_use]
    pub fn child(&self, name: &str) -> Self {
        let mut archive = self
            .archive
            .clone()
            .expect("child is only used inside an archive");
        archive.inner.push(name.to_owned());
        Self {
            base: self.base.clone(),
            archive: Some(archive),
        }
    }

    /// The directory on disk that stands for this place: the place itself,
    /// or the directory holding the archive.
    #[must_use]
    pub fn dir(&self) -> &Path {
        match &self.archive {
            None => &self.base,
            Some(_) => self.base.parent().unwrap_or(&self.base),
        }
    }

    /// The full name of an entry here, for showing or copying.
    #[must_use]
    pub fn child_display(&self, name: &OsStr) -> String {
        match &self.archive {
            None => self.base.join(name).to_string_lossy().into_owned(),
            Some(_) => format!("{self}/{}", name.to_string_lossy()),
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
        assert!(!root.has_parent());
    }

    #[test]
    fn parent_strips_last_component() {
        let location = Location::new(PathBuf::from("/home/user"));

        assert_eq!(
            location.parent(),
            Some(Location::new(PathBuf::from("/home")))
        );
        assert!(location.has_parent());
        assert_eq!(location.leaf_name(), Some(OsString::from("user")));
    }

    #[test]
    fn display_appends_archive_path() {
        let location = inside(&["d", "e"]);

        assert_eq!(location.to_string(), "/a/b/c.zip/d/e");
    }

    #[test]
    fn parent_inside_an_archive_drops_the_last_segment() {
        let location = inside(&["d", "e"]);

        assert_eq!(location.parent(), Some(inside(&["d"])));
        assert!(location.has_parent());
        assert_eq!(location.leaf_name(), Some(OsString::from("e")));
    }

    #[test]
    fn parent_of_the_archive_root_is_the_directory_holding_the_archive() {
        let root = inside(&[]);

        assert_eq!(root.parent(), Some(Location::new(PathBuf::from("/a/b"))));
        assert!(root.has_parent());
        assert_eq!(root.leaf_name(), Some(OsString::from("c.zip")));
        assert!(root.in_archive());
    }

    #[test]
    fn child_descends_inside_the_archive() {
        assert_eq!(inside(&[]).child("d"), inside(&["d"]));
    }

    #[test]
    fn dir_is_the_archive_parent_inside_an_archive() {
        assert_eq!(inside(&["d"]).dir(), Path::new("/a/b"));
        assert_eq!(
            Location::new(PathBuf::from("/a/b")).dir(),
            Path::new("/a/b")
        );
    }

    #[test]
    fn child_display_joins_with_a_slash_inside_an_archive() {
        assert_eq!(
            inside(&["d"]).child_display(OsStr::new("f.txt")),
            "/a/b/c.zip/d/f.txt"
        );
        assert_eq!(
            Location::new(PathBuf::from("/a/b")).child_display(OsStr::new("f.txt")),
            PathBuf::from("/a/b").join("f.txt").to_string_lossy()
        );
    }

    #[test]
    fn format_follows_the_extension_regardless_of_case() {
        assert_eq!(
            ArchiveFormat::from_name(OsStr::new("a.zip")),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(
            ArchiveFormat::from_name(OsStr::new("A.ZIP")),
            Some(ArchiveFormat::Zip)
        );
        assert_eq!(ArchiveFormat::from_name(OsStr::new("a.txt")), None);
        assert_eq!(ArchiveFormat::from_name(OsStr::new("zip")), None);
        assert_eq!(ArchiveFormat::from_name(OsStr::new(".zip")), None);
        assert_eq!(ArchiveFormat::from_name(OsStr::new("a.zip.bak")), None);
        assert_eq!(ArchiveFormat::Zip.extension(), "zip");
    }

    fn inside(segments: &[&str]) -> Location {
        Location {
            base: PathBuf::from("/a/b/c.zip"),
            archive: Some(ArchivePath {
                format: ArchiveFormat::Zip,
                inner: segments.iter().map(|s| (*s).to_owned()).collect(),
            }),
        }
    }
}
