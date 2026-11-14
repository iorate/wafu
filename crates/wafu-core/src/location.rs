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
    /// The archives this one is an entry of, outermost first. `base` is the
    /// file of the first of them, or of this archive when there are none.
    pub outer: Vec<OuterArchive>,
    /// `None` when the name did not tell; the I/O layer then reads the
    /// format off the content.
    pub format: Option<ArchiveFormat>,
    pub inner: Vec<String>,
}

/// An archive that holds the next one in an [`ArchivePath`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct OuterArchive {
    /// As [`ArchivePath::format`].
    pub format: Option<ArchiveFormat>,
    /// The path inside this archive of the archive it holds; never empty.
    pub entry: Vec<String>,
}

/// An archive format the I/O layer can read and write. Adding one means
/// adding a variant here, its extension below, and an implementation in the
/// I/O layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArchiveFormat {
    Zip,
    Tar,
    TarGz,
    TarBz2,
    TarXz,
}

impl ArchiveFormat {
    /// The format a file name stands for, by its extension. Case does not
    /// matter. `gz`, `bz2`, and `xz` count only after `.tar`.
    #[must_use]
    pub fn from_name(name: &OsStr) -> Option<Self> {
        let path = Path::new(name);
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        let compressed_tar = |format| {
            Path::new(path.file_stem()?)
                .extension()?
                .eq_ignore_ascii_case("tar")
                .then_some(format)
        };
        match extension.as_str() {
            "zip" => Some(Self::Zip),
            "tar" => Some(Self::Tar),
            "tgz" => Some(Self::TarGz),
            "tbz2" => Some(Self::TarBz2),
            "txz" => Some(Self::TarXz),
            "gz" => compressed_tar(Self::TarGz),
            "bz2" => compressed_tar(Self::TarBz2),
            "xz" => compressed_tar(Self::TarXz),
            _ => None,
        }
    }

    /// The extension given to a new archive, without the dot.
    #[must_use]
    pub fn extension(self) -> &'static str {
        match self {
            Self::Zip => "zip",
            Self::Tar => "tar",
            Self::TarGz => "tar.gz",
            Self::TarBz2 => "tar.bz2",
            Self::TarXz => "tar.xz",
        }
    }

    /// Whether the format can encrypt what it holds.
    #[must_use]
    pub fn takes_password(self) -> bool {
        matches!(self, Self::Zip)
    }

    /// The name without its archive extension: `a` for `a.tar.gz` as well as
    /// for `a.tgz`. For other names, the part before the last dot.
    #[must_use]
    pub fn stem_of(name: &OsStr) -> &OsStr {
        let stem = Path::new(name).file_stem().unwrap_or(name);
        match Self::from_name(name) {
            Some(Self::TarGz | Self::TarBz2 | Self::TarXz)
                if Path::new(stem)
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("tar")) =>
            {
                Path::new(stem).file_stem().unwrap_or(stem)
            }
            _ => stem,
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
    pub fn archive_root(base: PathBuf, format: Option<ArchiveFormat>) -> Self {
        Self {
            base,
            archive: Some(ArchivePath {
                outer: Vec::new(),
                format,
                inner: Vec::new(),
            }),
        }
    }

    /// The root of the archive that is the entry `name` in this place inside
    /// an archive.
    #[must_use]
    pub fn nested_root(&self, name: &str, format: Option<ArchiveFormat>) -> Self {
        let archive = self
            .archive
            .as_ref()
            .expect("nested_root is only used inside an archive");
        let mut entry = archive.inner.clone();
        entry.push(name.to_owned());
        let mut outer = archive.outer.clone();
        outer.push(OuterArchive {
            format: archive.format,
            entry,
        });
        Self {
            base: self.base.clone(),
            archive: Some(ArchivePath {
                outer,
                format,
                inner: Vec::new(),
            }),
        }
    }

    /// The root of the innermost archive this place is in, or `None` outside
    /// an archive.
    #[must_use]
    pub fn innermost_root(&self) -> Option<Self> {
        let archive = self.archive.as_ref()?;
        Some(Self {
            base: self.base.clone(),
            archive: Some(ArchivePath {
                outer: archive.outer.clone(),
                format: archive.format,
                inner: Vec::new(),
            }),
        })
    }

    #[must_use]
    pub fn in_archive(&self) -> bool {
        self.archive.is_some()
    }

    /// Whether `parent` returns a place, without building it.
    #[must_use]
    pub fn has_parent(&self) -> bool {
        match &self.archive {
            Some(archive) if !archive.inner.is_empty() || !archive.outer.is_empty() => true,
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
                        outer: archive.outer.clone(),
                        format: archive.format,
                        inner,
                    }),
                })
            }
            Some(archive) if !archive.outer.is_empty() => {
                let mut outer = archive.outer.clone();
                let holder = outer.pop()?;
                let mut inner = holder.entry;
                inner.pop();
                Some(Self {
                    base: self.base.clone(),
                    archive: Some(ArchivePath {
                        outer,
                        format: holder.format,
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
            Some(archive) if !archive.outer.is_empty() => archive
                .outer
                .last()
                .and_then(|holder| holder.entry.last())
                .map(OsString::from),
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
            let entries = archive.outer.iter().flat_map(|holder| &holder.entry);
            for segment in entries.chain(&archive.inner) {
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
    fn parent_inside_an_archive_drops_the_last_segment() {
        let location = inside(&["d", "e"]);

        assert_eq!(location.parent(), Some(inside(&["d"])));
        assert!(location.has_parent());
        assert_eq!(location.leaf_name(), Some(OsString::from("e")));
    }

    #[test]
    fn parent_of_an_archive_root_is_the_directory_holding_the_archive() {
        let root = inside(&[]);

        assert_eq!(root.parent(), Some(Location::new(PathBuf::from("/a/b"))));
        assert!(root.has_parent());
        assert_eq!(root.leaf_name(), Some(OsString::from("c.zip")));
    }

    #[test]
    fn parent_of_a_nested_root_is_the_directory_holding_it_in_the_outer_archive() {
        let root = nested(&[]);

        assert_eq!(root.parent(), Some(inside(&["d"])));
        assert!(root.has_parent());
        assert_eq!(root.leaf_name(), Some(OsString::from("n.tar.gz")));
    }

    #[test]
    fn parent_inside_a_nested_archive_stays_in_it() {
        assert_eq!(nested(&["x", "y"]).parent(), Some(nested(&["x"])));
    }

    #[test]
    fn nested_root_records_the_outer_archive_and_its_entry() {
        let root = nested(&[]).nested_root("m.tar", Some(ArchiveFormat::Tar));

        let archive = root.archive.unwrap();
        assert_eq!(
            archive.outer,
            [
                OuterArchive {
                    format: Some(ArchiveFormat::Zip),
                    entry: vec!["d".to_owned(), "n.tar.gz".to_owned()],
                },
                OuterArchive {
                    format: Some(ArchiveFormat::TarGz),
                    entry: vec!["m.tar".to_owned()],
                },
            ]
        );
        assert_eq!(archive.format, Some(ArchiveFormat::Tar));
        assert_eq!(archive.inner, Vec::<String>::new());
    }

    #[test]
    fn innermost_root_keeps_the_outer_archives() {
        assert_eq!(nested(&["x", "y"]).innermost_root(), Some(nested(&[])));
        assert_eq!(inside(&["d"]).innermost_root(), Some(inside(&[])));
        assert_eq!(Location::new(PathBuf::from("/a")).innermost_root(), None);
    }

    #[test]
    fn a_filesystem_root_has_no_parent() {
        let root = Location::new(PathBuf::from("/"));

        assert_eq!(root.parent(), None);
        assert!(!root.has_parent());
    }

    #[test]
    fn child_appends_a_segment_inside_an_archive() {
        assert_eq!(inside(&["d"]).child("e"), inside(&["d", "e"]));
    }

    #[test]
    fn dir_inside_an_archive_is_the_directory_holding_the_archive() {
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
    fn display_of_a_nested_archive_goes_through_the_outer_entries() {
        assert_eq!(nested(&["x"]).to_string(), "/a/b/c.zip/d/n.tar.gz/x");
    }

    #[test]
    fn format_follows_the_extension() {
        for (name, format) in [
            ("a.zip", Some(ArchiveFormat::Zip)),
            ("A.ZIP", Some(ArchiveFormat::Zip)),
            ("a.txt", None),
            ("zip", None),
            (".zip", None),
            ("a.zip.bak", None),
            ("a.tar", Some(ArchiveFormat::Tar)),
            ("a.tar.gz", Some(ArchiveFormat::TarGz)),
            ("a.TAR.GZ", Some(ArchiveFormat::TarGz)),
            ("a.tgz", Some(ArchiveFormat::TarGz)),
            ("a.tar.bz2", Some(ArchiveFormat::TarBz2)),
            ("a.tbz2", Some(ArchiveFormat::TarBz2)),
            ("a.tar.xz", Some(ArchiveFormat::TarXz)),
            ("a.txz", Some(ArchiveFormat::TarXz)),
            ("a.gz", None),
            ("a.xz", None),
            ("a.bz2", None),
            (".tar.gz", None),
            ("tar.gz", None),
            ("a.tar.", None),
        ] {
            assert_eq!(ArchiveFormat::from_name(OsStr::new(name)), format, "{name}");
        }
        assert_eq!(ArchiveFormat::Zip.extension(), "zip");
    }

    #[test]
    fn stem_drops_the_whole_archive_extension() {
        let stem = |name| ArchiveFormat::stem_of(OsStr::new(name));

        assert_eq!(stem("a.tar.gz"), "a");
        assert_eq!(stem("a.TAR.GZ"), "a");
        assert_eq!(stem(".tar.gz"), ".tar");
        assert_eq!(stem("a.b.tgz"), "a.b");
        assert_eq!(stem("a.gz"), "a");
    }

    fn inside(segments: &[&str]) -> Location {
        Location {
            base: PathBuf::from("/a/b/c.zip"),
            archive: Some(ArchivePath {
                outer: Vec::new(),
                format: Some(ArchiveFormat::Zip),
                inner: strings(segments),
            }),
        }
    }

    // `/a/b/c.zip/d/n.tar.gz`, then `segments` inside it.
    fn nested(segments: &[&str]) -> Location {
        let mut location = inside(&["d"]).nested_root("n.tar.gz", Some(ArchiveFormat::TarGz));
        if let Some(archive) = &mut location.archive {
            archive.inner = strings(segments);
        }
        location
    }

    fn strings(segments: &[&str]) -> Vec<String> {
        segments.iter().map(|s| (*s).to_owned()).collect()
    }
}
