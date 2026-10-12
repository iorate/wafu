use std::path::{Component, Path, PathBuf, Prefix};

/// Replaces a leading `~` with the home directory. `~user` is left alone.
#[must_use]
pub fn expand_tilde(text: &str, home: &Path) -> PathBuf {
    if text == "~" {
        return home.to_path_buf();
    }
    match text.strip_prefix("~/").or_else(|| text.strip_prefix("~\\")) {
        Some(rest) => home.join(rest),
        None => PathBuf::from(text),
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

/// Folds `.` and `..` lexically. Symbolic links are not followed, so the
/// result names the path the user typed rather than where it leads.
#[must_use]
pub fn normalize(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {
                result.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                let is_normal =
                    matches!(result.components().next_back(), Some(Component::Normal(_)));
                if is_normal {
                    result.pop();
                }
            }
        }
    }
    result
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
            expand_tilde("~\\docs", home),
            Path::new("/home/user").join("docs")
        );
    }

    #[test]
    fn tilde_user_and_other_text_are_left_alone() {
        let home = Path::new("/home/user");

        assert_eq!(expand_tilde("~other", home), PathBuf::from("~other"));
        assert_eq!(expand_tilde("docs", home), PathBuf::from("docs"));
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
