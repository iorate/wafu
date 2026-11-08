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
    /// `None` when the name did not tell; the I/O layer then reads the
    /// format off the content.
    pub format: Option<ArchiveFormat>,
    pub inner: Vec<String>,
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
    fn parent_inside_an_archive_drops_the_last_segment() {
        let location = inside(&["d", "e"]);

        assert_eq!(location.parent(), Some(inside(&["d"])));
        assert!(location.has_parent());
        assert_eq!(location.leaf_name(), Some(OsString::from("e")));
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

    #[test]
    fn tar_formats_come_from_one_or_two_extensions() {
        let format = |name| ArchiveFormat::from_name(OsStr::new(name));

        assert_eq!(format("a.tar"), Some(ArchiveFormat::Tar));
        assert_eq!(format("a.tar.gz"), Some(ArchiveFormat::TarGz));
        assert_eq!(format("a.TAR.GZ"), Some(ArchiveFormat::TarGz));
        assert_eq!(format("a.tgz"), Some(ArchiveFormat::TarGz));
        assert_eq!(format("a.tar.bz2"), Some(ArchiveFormat::TarBz2));
        assert_eq!(format("a.tbz2"), Some(ArchiveFormat::TarBz2));
        assert_eq!(format("a.tar.xz"), Some(ArchiveFormat::TarXz));
        assert_eq!(format("a.txz"), Some(ArchiveFormat::TarXz));
        assert_eq!(format("a.gz"), None);
        assert_eq!(format("a.xz"), None);
        assert_eq!(format(".tar.gz"), None);
        assert_eq!(format("tar.gz"), None);
        assert_eq!(format("a.TAR.XZ"), Some(ArchiveFormat::TarXz));
        assert_eq!(format("A.TGZ"), Some(ArchiveFormat::TarGz));
        assert_eq!(format("a.tar.gz.bak"), None);
        assert_eq!(format("a.tar."), None);
        assert_eq!(format(".tar"), None);
        assert_eq!(format("a.bz2"), None);
        assert_eq!(ArchiveFormat::TarGz.extension(), "tar.gz");
        assert!(ArchiveFormat::Zip.takes_password());
        assert!(!ArchiveFormat::TarGz.takes_password());
    }

    #[test]
    fn stem_drops_the_whole_archive_extension() {
        let stem = |name| ArchiveFormat::stem_of(OsStr::new(name));

        assert_eq!(stem("a.tar.gz"), "a");
        assert_eq!(stem("a.TAR.GZ"), "a");
        assert_eq!(stem("a.tar"), "a");
        assert_eq!(stem(".tar.gz"), ".tar");
        assert_eq!(stem("a.b.tgz"), "a.b");
        assert_eq!(stem("a.zip"), "a");
        assert_eq!(stem("a.txt"), "a");
        assert_eq!(stem("a.gz"), "a");
        assert_eq!(stem("a"), "a");
        assert_eq!(stem(".hidden"), ".hidden");
    }

    fn inside(segments: &[&str]) -> Location {
        Location {
            base: PathBuf::from("/a/b/c.zip"),
            archive: Some(ArchivePath {
                format: Some(ArchiveFormat::Zip),
                inner: segments.iter().map(|s| (*s).to_owned()).collect(),
            }),
        }
    }
}
