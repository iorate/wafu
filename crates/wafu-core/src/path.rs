use std::ffi::{OsStr, OsString};
use std::path::{Component, MAIN_SEPARATOR_STR, Path, PathBuf, Prefix};

/// Replaces a leading `~` with the home directory. `~user` is left alone.
#[must_use]
pub fn expand_tilde(text: &str, home: &Path) -> PathBuf {
    match text.strip_prefix('~') {
        Some("") => home.to_path_buf(),
        Some(rest) if rest.starts_with('/') || (cfg!(windows) && rest.starts_with('\\')) => {
            let mut path = home.as_os_str().to_owned();
            for part in rest
                .split(std::path::is_separator)
                .filter(|part| !part.is_empty())
            {
                path.push(std::path::MAIN_SEPARATOR_STR);
                path.push(part);
            }
            PathBuf::from(path)
        }
        _ => PathBuf::from(text),
    }
}

/// Turns typed text into a place to go: `~` is expanded, a relative path is
/// taken from `base`, and `.` and `..` are folded without touching the file system.
#[must_use]
pub fn resolve_input(text: &str, home: &Path, base: &Path) -> PathBuf {
    let path = expand_tilde(text, home);
    let path = if path.is_relative() {
        base.join(path)
    } else {
        path
    };
    normalize(&path)
}

// Folds `.` and `..` lexically. Symbolic links are not followed, so the
// result names the path the user typed rather than where it leads.
#[must_use]
fn normalize(path: &Path) -> PathBuf {
    let mut prefix = None;
    let mut has_root = false;
    let mut parts: Vec<&OsStr> = Vec::new();
    for component in path.components() {
        match component {
            Component::Prefix(value) => prefix = Some(value.as_os_str()),
            Component::RootDir => has_root = true,
            Component::Normal(part) => parts.push(part),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop();
            }
        }
    }
    let mut result = OsString::new();
    if let Some(prefix) = prefix {
        result.push(prefix);
    }
    if has_root {
        result.push(MAIN_SEPARATOR_STR);
    }
    for (index, part) in parts.into_iter().enumerate() {
        if index > 0 {
            result.push(MAIN_SEPARATOR_STR);
        }
        result.push(part);
    }
    PathBuf::from(result)
}

/// The root of the path: `/`, `C:\`, or `\\server\share\`. `None` for an empty path.
#[must_use]
pub fn root_of(path: &Path) -> Option<PathBuf> {
    let root = path.ancestors().last()?;
    (!root.as_os_str().is_empty()).then(|| root.to_path_buf())
}

/// The upper-case drive letter of a Windows path such as `C:\Users`.
#[must_use]
pub fn drive_letter(path: &Path) -> Option<char> {
    match path.components().next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                Some(char::from(letter).to_ascii_uppercase())
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tilde_alone_and_tilde_slash_expand_to_home() {
        let home = Path::new("/home/user");

        assert_eq!(expand_tilde("~", home), PathBuf::from("/home/user"));
        assert_eq!(
            expand_tilde("~/docs", home),
            Path::new("/home/user").join("docs")
        );
        assert_eq!(
            expand_tilde("~//docs", home),
            Path::new("/home/user").join("docs")
        );
    }

    #[cfg(windows)]
    #[test]
    fn tilde_backslash_expands_to_home_on_windows() {
        let home = Path::new("/home/user");

        assert_eq!(
            expand_tilde("~\\docs", home),
            Path::new("/home/user").join("docs")
        );
        assert_eq!(
            expand_tilde("~/\\docs", home),
            Path::new("/home/user").join("docs")
        );
    }

    #[test]
    fn tilde_slash_keeps_a_drive_prefix_under_home() {
        let home = Path::new("/home/user");

        let result = expand_tilde("~/C:/docs", home);

        assert!(result.starts_with(home));
        assert!(result.ends_with("docs"));
    }

    #[test]
    fn tilde_user_and_other_text_are_left_alone() {
        let home = Path::new("/home/user");

        assert_eq!(expand_tilde("~other", home), PathBuf::from("~other"));
        assert_eq!(expand_tilde("docs", home), PathBuf::from("docs"));
    }

    #[cfg(not(windows))]
    #[test]
    fn tilde_backslash_is_left_alone_off_windows() {
        let home = Path::new("/home/user");

        assert_eq!(expand_tilde("~\\docs", home), PathBuf::from("~\\docs"));
    }

    #[test]
    fn relative_input_is_taken_from_the_base() {
        let home = Path::new("/home/user");
        let base = Path::new("/var/log");

        assert_eq!(
            resolve_input("nginx", home, base),
            Path::new("/var/log").join("nginx")
        );
        assert_eq!(resolve_input("..", home, base), PathBuf::from("/var"));
        assert_eq!(resolve_input("~", home, base), PathBuf::from("/home/user"));
    }

    #[test]
    fn resolve_input_keeps_a_drive_like_part_under_home() {
        let home = Path::new("/home/user");
        let base = Path::new("/var/log");

        let result = resolve_input("~/C:/docs", home, base);

        assert!(result.starts_with(home));
        assert!(result.ends_with("docs"));
    }

    #[test]
    fn normalize_folds_dot_and_dot_dot() {
        assert_eq!(
            normalize(Path::new("/a/./b/../c")),
            Path::new("/a").join("c")
        );
        assert_eq!(normalize(Path::new("/a/b/../../..")), PathBuf::from("/"));
        assert_eq!(normalize(Path::new("/..")), PathBuf::from("/"));
        assert_eq!(normalize(Path::new("a/../b")), PathBuf::from("b"));
    }

    #[test]
    fn normalize_drops_trailing_and_repeated_separators() {
        assert_eq!(
            normalize(Path::new("/home//user/")),
            Path::new("/home").join("user")
        );
    }

    #[cfg(windows)]
    #[test]
    fn normalize_unifies_separators_on_windows() {
        assert!(
            !normalize(Path::new("/home//user/"))
                .to_string_lossy()
                .contains('/')
        );
        assert!(
            !normalize(Path::new("C:/Users/me"))
                .to_string_lossy()
                .contains('/')
        );
    }

    #[cfg(windows)]
    #[test]
    fn normalize_keeps_a_windows_prefix() {
        for text in [r"C:\Users\me", r"\\server\share\x", r"\\?\C:\x"] {
            assert_eq!(normalize(Path::new(text)), PathBuf::from(text));
        }
    }

    #[test]
    fn normalize_keeps_a_drive_like_part_under_its_parent() {
        let result = normalize(Path::new("/home/user/C:/docs"));

        assert!(result.starts_with("/home/user"));
        assert!(result.ends_with("docs"));
    }

    #[test]
    fn root_of_finds_the_top_and_rejects_empty_paths() {
        assert_eq!(root_of(Path::new("/a/b")), Some(PathBuf::from("/")));
        assert_eq!(root_of(Path::new("/")), Some(PathBuf::from("/")));
        assert_eq!(root_of(Path::new("")), None);
    }

    #[test]
    fn unix_paths_have_no_drive_letter() {
        assert_eq!(drive_letter(Path::new("/home/user")), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_paths_report_their_drive_letter() {
        assert_eq!(drive_letter(Path::new(r"c:\Users")), Some('C'));
        assert_eq!(drive_letter(Path::new(r"D:\")), Some('D'));
        assert_eq!(drive_letter(Path::new(r"\\?\E:\x")), Some('E'));
        assert_eq!(drive_letter(Path::new(r"\\server\share\x")), None);
    }

    #[cfg(windows)]
    #[test]
    fn root_of_a_windows_path_is_the_drive_root() {
        assert_eq!(
            root_of(Path::new(r"C:\Users\me")),
            Some(PathBuf::from(r"C:\"))
        );
        assert_eq!(
            root_of(Path::new(r"\\server\share\x")),
            Some(PathBuf::from(r"\\server\share\"))
        );
    }
}
