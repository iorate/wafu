use std::ffi::OsString;
use std::fmt;
use std::time::SystemTime;

/// One item of a directory listing, as read by the I/O layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: OsString,
    pub kind: EntryKind,
    pub size: u64,
    pub modified: Option<Timestamp>,
    pub hidden: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File,
    /// A symbolic link. `target` is `None` when the link is broken.
    Symlink {
        target: Option<TargetKind>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Dir,
    File,
}

/// A modification time, both as an instant for sorting and as local civil time for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp {
    pub at: SystemTime,
    pub local: LocalDateTime,
}

/// Civil date and time in the local time zone, already resolved by the I/O layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LocalDateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl Entry {
    /// Splits the name into the part shown in the name column and the extension column.
    #[must_use]
    pub fn split_name(&self) -> (String, String) {
        let name = self.name.to_string_lossy();
        if self.kind.is_dir_like() {
            return (name.into_owned(), String::new());
        }
        // A leading dot alone does not start an extension.
        let dot = name
            .char_indices()
            .skip(1)
            .filter(|&(_, c)| c == '.')
            .map(|(index, _)| index)
            .last();
        match dot {
            Some(index) => (name[..index].to_owned(), name[index + 1..].to_owned()),
            None => (name.into_owned(), String::new()),
        }
    }
}

impl EntryKind {
    #[must_use]
    pub fn is_broken_link(self) -> bool {
        self == Self::Symlink { target: None }
    }

    #[must_use]
    pub fn is_dir_like(self) -> bool {
        matches!(
            self,
            Self::Dir
                | Self::Symlink {
                    target: Some(TargetKind::Dir)
                }
        )
    }
}

impl fmt::Display for LocalDateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}/{:02}/{:02} {:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_name_at_last_dot() {
        assert_split("a.tar.gz", EntryKind::File, "a.tar", "gz");
    }

    #[test]
    fn split_name_without_dot_has_no_extension() {
        assert_split("noext", EntryKind::File, "noext", "");
    }

    #[test]
    fn split_name_with_only_leading_dot_has_no_extension() {
        assert_split(".gitignore", EntryKind::File, ".gitignore", "");
    }

    #[test]
    fn split_name_with_leading_dot_and_another_dot() {
        assert_split(".bashrc.bak", EntryKind::File, ".bashrc", "bak");
    }

    #[test]
    fn split_name_of_directory_has_no_extension() {
        assert_split("conf.d", EntryKind::Dir, "conf.d", "");
    }

    #[test]
    fn split_name_of_symlink_to_directory_has_no_extension() {
        let kind = EntryKind::Symlink {
            target: Some(TargetKind::Dir),
        };

        assert_split("link.d", kind, "link.d", "");
    }

    #[test]
    fn split_name_of_broken_symlink_is_split_like_a_file() {
        let kind = EntryKind::Symlink { target: None };

        assert_split("broken.lnk", kind, "broken", "lnk");
    }

    #[test]
    fn split_name_with_multibyte_first_char() {
        assert_split("日本.txt", EntryKind::File, "日本", "txt");
    }

    #[test]
    fn split_name_of_empty_name() {
        assert_split("", EntryKind::File, "", "");
    }

    #[test]
    fn local_date_time_is_zero_padded() {
        let time = LocalDateTime {
            year: 2026,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 5,
        };

        assert_eq!(time.to_string(), "2026/01/02 03:04:05");
    }

    fn assert_split(name: &str, kind: EntryKind, expected_name: &str, expected_extension: &str) {
        let entry = Entry {
            name: OsString::from(name),
            kind,
            size: 0,
            modified: None,
            hidden: false,
        };

        let (name, extension) = entry.split_name();

        assert_eq!(
            (name.as_str(), extension.as_str()),
            (expected_name, expected_extension)
        );
    }
}
