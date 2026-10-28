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
    /// What the permission columns are made from; `None` when not known.
    pub permissions: Option<Permissions>,
    /// Where a symbolic link points, when known.
    pub link_target: Option<OsString>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Dir,
    File {
        executable: bool,
    },
    /// A symbolic link. `target` is `None` when the link is broken.
    Symlink {
        target: Option<TargetKind>,
    },
    /// A FIFO, socket, or device, whose content cannot be read as a file.
    Special(SpecialKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permissions {
    /// Unix permission bits, with setuid, setgid, and sticky.
    Mode(u32),
    /// Windows attributes, approximated as `ls` does on MSYS2.
    Attributes { readonly: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Dir,
    File,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecialKind {
    Fifo,
    Socket,
    BlockDevice,
    CharDevice,
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

    /// The ten characters `ls -l` starts a line with: the kind and the permissions.
    #[must_use]
    pub fn permissions_text(&self) -> String {
        let kind = match self.kind {
            EntryKind::Dir => 'd',
            EntryKind::File { .. } => '-',
            EntryKind::Symlink { .. } => 'l',
            EntryKind::Special(SpecialKind::Fifo) => 'p',
            EntryKind::Special(SpecialKind::Socket) => 's',
            EntryKind::Special(SpecialKind::BlockDevice) => 'b',
            EntryKind::Special(SpecialKind::CharDevice) => 'c',
        };
        let bits = match self.permissions {
            Some(Permissions::Mode(mode)) => mode_text(mode),
            Some(Permissions::Attributes { readonly }) => attributes_text(self.kind, readonly),
            None => "?????????".to_owned(),
        };
        format!("{kind}{bits}")
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

    #[must_use]
    pub fn is_special(self) -> bool {
        matches!(self, Self::Special(_))
    }

    /// The mark `ls -F` would print after the name, or an empty string.
    #[must_use]
    pub fn marker(self) -> &'static str {
        match self {
            Self::Dir => "/",
            Self::File { executable: true } => "*",
            Self::File { executable: false }
            | Self::Special(SpecialKind::BlockDevice | SpecialKind::CharDevice) => "",
            Self::Symlink { .. } => "@",
            Self::Special(SpecialKind::Fifo) => "|",
            Self::Special(SpecialKind::Socket) => "=",
        }
    }
}

fn mode_text(mode: u32) -> String {
    let mut text = String::with_capacity(9);
    for (shift, special, special_char) in [(6, 0o4000, 's'), (3, 0o2000, 's'), (0, 0o1000, 't')] {
        let bits = mode >> shift;
        text.push(if bits & 0o4 == 0 { '-' } else { 'r' });
        text.push(if bits & 0o2 == 0 { '-' } else { 'w' });
        text.push(match (mode & special != 0, bits & 0o1 != 0) {
            (true, true) => special_char,
            (true, false) => special_char.to_ascii_uppercase(),
            (false, true) => 'x',
            (false, false) => '-',
        });
    }
    text
}

fn attributes_text(kind: EntryKind, readonly: bool) -> String {
    let base = match kind {
        EntryKind::Symlink { .. } => return "rwxrwxrwx".to_owned(),
        EntryKind::Dir | EntryKind::File { executable: true } => "rwxr-xr-x",
        EntryKind::File { executable: false } | EntryKind::Special(_) => "rw-r--r--",
    };
    if readonly {
        base.replace('w', "-")
    } else {
        base.to_owned()
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
        assert_split(
            "a.tar.gz",
            EntryKind::File { executable: false },
            "a.tar",
            "gz",
        );
    }

    #[test]
    fn split_name_without_dot_has_no_extension() {
        assert_split("noext", EntryKind::File { executable: false }, "noext", "");
    }

    #[test]
    fn split_name_with_only_leading_dot_has_no_extension() {
        assert_split(
            ".gitignore",
            EntryKind::File { executable: false },
            ".gitignore",
            "",
        );
    }

    #[test]
    fn split_name_with_leading_dot_and_another_dot() {
        assert_split(
            ".bashrc.bak",
            EntryKind::File { executable: false },
            ".bashrc",
            "bak",
        );
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
        assert_split(
            "日本.txt",
            EntryKind::File { executable: false },
            "日本",
            "txt",
        );
    }

    #[test]
    fn split_name_of_empty_name() {
        assert_split("", EntryKind::File { executable: false }, "", "");
    }

    #[test]
    fn markers_follow_ls_f() {
        let link = EntryKind::Symlink {
            target: Some(TargetKind::Dir),
        };

        assert_eq!(EntryKind::Dir.marker(), "/");
        assert_eq!(EntryKind::File { executable: true }.marker(), "*");
        assert_eq!(EntryKind::File { executable: false }.marker(), "");
        assert_eq!(link.marker(), "@");
        assert_eq!(EntryKind::Symlink { target: None }.marker(), "@");
        assert_eq!(EntryKind::Special(SpecialKind::Fifo).marker(), "|");
        assert_eq!(EntryKind::Special(SpecialKind::Socket).marker(), "=");
        assert_eq!(EntryKind::Special(SpecialKind::BlockDevice).marker(), "");
        assert_eq!(EntryKind::Special(SpecialKind::CharDevice).marker(), "");
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

    #[test]
    fn permissions_text_follows_ls_l_for_modes() {
        let file = EntryKind::File { executable: false };
        let cases = [
            (file, 0o644, "-rw-r--r--"),
            (EntryKind::Dir, 0o755, "drwxr-xr-x"),
            (file, 0o4755, "-rwsr-xr-x"),
            (file, 0o2644, "-rw-r-Sr--"),
            (EntryKind::Dir, 0o1777, "drwxrwxrwt"),
            (EntryKind::Symlink { target: None }, 0o777, "lrwxrwxrwx"),
            (EntryKind::Special(SpecialKind::Fifo), 0o600, "prw-------"),
            (EntryKind::Special(SpecialKind::Socket), 0o600, "srw-------"),
            (
                EntryKind::Special(SpecialKind::BlockDevice),
                0o660,
                "brw-rw----",
            ),
            (
                EntryKind::Special(SpecialKind::CharDevice),
                0o666,
                "crw-rw-rw-",
            ),
        ];
        for (kind, mode, expected) in cases {
            let entry = entry(kind, Some(Permissions::Mode(mode)));

            assert_eq!(entry.permissions_text(), expected, "{mode:o}");
        }
    }

    #[test]
    fn permissions_text_approximates_windows_attributes() {
        let file = EntryKind::File { executable: false };
        let exe = EntryKind::File { executable: true };
        let link = EntryKind::Symlink {
            target: Some(TargetKind::File),
        };
        let cases = [
            (file, false, "-rw-r--r--"),
            (file, true, "-r--r--r--"),
            (exe, false, "-rwxr-xr-x"),
            (exe, true, "-r-xr-xr-x"),
            (EntryKind::Dir, false, "drwxr-xr-x"),
            (EntryKind::Dir, true, "dr-xr-xr-x"),
            (link, true, "lrwxrwxrwx"),
        ];
        for (kind, readonly, expected) in cases {
            let entry = entry(kind, Some(Permissions::Attributes { readonly }));

            assert_eq!(entry.permissions_text(), expected, "{kind:?} {readonly}");
        }
    }

    #[test]
    fn permissions_text_shows_question_marks_when_unknown() {
        let entry = entry(EntryKind::Dir, None);

        assert_eq!(entry.permissions_text(), "d?????????");
    }

    fn entry(kind: EntryKind, permissions: Option<Permissions>) -> Entry {
        Entry {
            name: OsString::from("x"),
            kind,
            size: 0,
            modified: None,
            hidden: false,
            permissions,
            link_target: None,
        }
    }

    fn assert_split(name: &str, kind: EntryKind, expected_name: &str, expected_extension: &str) {
        let entry = Entry {
            name: OsString::from(name),
            kind,
            size: 0,
            modified: None,
            hidden: false,
            permissions: None,
            link_target: None,
        };

        let (name, extension) = entry.split_name();

        assert_eq!(
            (name.as_str(), extension.as_str()),
            (expected_name, expected_extension)
        );
    }
}
