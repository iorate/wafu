use std::path::Path;

use ignore::{Error, WalkBuilder};
use wafu_core::{FindQuery, IoError, OperationEvent, Outcome};

use super::Worker;
use crate::read_dir::{has_hidden_attribute, has_hidden_name};

impl Worker<'_> {
    // The walk is serial, so that it stops right at the limit or a
    // cancellation; a parallel one goes on delivering entries after it is
    // told to quit.
    pub(super) fn find(&mut self, query: &FindQuery) -> Outcome {
        let show_hidden = query.show_hidden;
        // `ignore`'s own hidden filter misses macOS's `UF_HIDDEN`; the
        // listing's test is used so that a find skips what the pane hides.
        let walk = WalkBuilder::new(&query.dir)
            .standard_filters(false)
            .filter_entry(move |entry| {
                show_hidden
                    || entry.depth() == 0
                    || !(has_hidden_name(entry.file_name())
                        || cfg!(any(windows, target_os = "macos"))
                            && has_hidden_attribute(entry.metadata().ok().as_ref()))
            })
            .build();
        let mut paths = Vec::new();
        let mut truncated = false;
        for result in walk {
            let entry = match result {
                Ok(entry) => entry,
                Err(error) => {
                    let path = error_path(&error).unwrap_or(&query.dir).to_path_buf();
                    let message = error
                        .io_error()
                        .map_or_else(|| error.to_string(), ToString::to_string);
                    (self.report)(OperationEvent::Skipped {
                        path,
                        error: IoError::new(message),
                    });
                    continue;
                }
            };
            if entry.depth() == 0 {
                continue;
            }
            let Ok(relative) = entry.path().strip_prefix(&query.dir) else {
                continue;
            };
            if self
                .begin(entry.path().parent().unwrap_or(&query.dir))
                .is_err()
            {
                return Outcome::Cancelled;
            }
            let is_dir = entry
                .file_type()
                .is_some_and(|file_type| file_type.is_dir());
            if query.pattern.matches_path(relative, is_dir) {
                if paths.len() == query.max_results {
                    truncated = true;
                    break;
                }
                paths.push(entry.into_path());
                self.done += 1;
            }
        }
        paths.sort();
        Outcome::Found { paths, truncated }
    }
}

fn error_path(error: &Error) -> Option<&Path> {
    match error {
        Error::WithPath { path, .. } => Some(path),
        Error::WithDepth { err, .. } | Error::WithLineNumber { err, .. } => error_path(err),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::sync::Mutex;
    use std::sync::mpsc;

    use tempfile::TempDir;
    use wafu_core::{Filter, OperationKind, OperationSpec, Reply, Sources};

    use super::*;
    use crate::archive::NestedArchives;
    use crate::operation::run_operation;
    use crate::watch::tests::symlink;

    #[test]
    fn finds_files_and_directories_below_the_directory_sorted_by_path() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("b").join("c")).unwrap();
        fs::write(root.join("b").join("c").join("x.txt"), b"").unwrap();
        fs::write(root.join("a.txt"), b"").unwrap();
        fs::write(root.join("b.md"), b"").unwrap();

        let (outcome, _) = find(root, "*", false, 10_000, []);

        assert_eq!(
            outcome,
            Outcome::Found {
                paths: vec![
                    root.join("a.txt"),
                    root.join("b"),
                    root.join("b").join("c"),
                    root.join("b").join("c").join("x.txt"),
                    root.join("b.md"),
                ],
                truncated: false,
            }
        );
    }

    #[test]
    fn hidden_entries_are_found_and_searched_only_when_shown() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir(root.join(".hidden")).unwrap();
        fs::write(root.join(".hidden").join("a"), b"").unwrap();
        fs::write(root.join(".b"), b"").unwrap();
        fs::write(root.join("c"), b"").unwrap();

        let (hidden, _) = find(root, "*", false, 10_000, []);
        let (shown, _) = find(root, "*", true, 10_000, []);

        assert_eq!(
            hidden,
            Outcome::Found {
                paths: vec![root.join("c")],
                truncated: false,
            }
        );
        assert_eq!(
            shown,
            Outcome::Found {
                paths: vec![
                    root.join(".b"),
                    root.join(".hidden"),
                    root.join(".hidden").join("a"),
                    root.join("c"),
                ],
                truncated: false,
            }
        );
    }

    #[test]
    fn a_link_is_found_but_not_followed() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        fs::create_dir(root.join("d")).unwrap();
        fs::write(root.join("d").join("a"), b"").unwrap();
        if !symlink(&root.join("d"), &root.join("link")) {
            return;
        }

        let (outcome, _) = find(root, "*", false, 10_000, []);

        assert_eq!(
            outcome,
            Outcome::Found {
                paths: vec![root.join("d"), root.join("d").join("a"), root.join("link")],
                truncated: false,
            }
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_directory_is_skipped_and_the_rest_searched() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let locked = root.join("locked");
        fs::create_dir(&locked).unwrap();
        fs::write(locked.join("a"), b"").unwrap();
        fs::write(root.join("b"), b"").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        // A user who can read it anyway (root) has nothing to check.
        let readable = fs::read_dir(&locked).is_ok();

        let (outcome, events) = find(root, "*", false, 10_000, []);
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();

        if readable {
            return;
        }
        assert_eq!(
            outcome,
            Outcome::Found {
                paths: vec![root.join("b"), locked.clone()],
                truncated: false,
            }
        );
        let skipped: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                OperationEvent::Skipped { path, .. } => Some(path),
                _ => None,
            })
            .collect();
        assert_eq!(skipped, [&locked]);
    }

    #[test]
    fn stops_at_the_limit() {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        for name in ["a", "b", "c"] {
            fs::write(root.join(name), b"").unwrap();
        }

        let (all, _) = find(root, "*", false, 3, []);
        let (cut, _) = find(root, "*", false, 2, []);

        assert!(matches!(all, Outcome::Found { paths, truncated: false } if paths.len() == 3));
        assert!(matches!(cut, Outcome::Found { paths, truncated: true } if paths.len() == 2));
    }

    #[test]
    fn a_cancelled_find_reports_nothing_found() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a"), b"").unwrap();

        let (outcome, _) = find(dir.path(), "*", false, 10_000, [Reply::Cancel]);

        assert_eq!(outcome, Outcome::Cancelled);
    }

    fn find(
        dir: &Path,
        pattern: &str,
        show_hidden: bool,
        max_results: usize,
        replies: impl IntoIterator<Item = Reply>,
    ) -> (Outcome, Vec<OperationEvent>) {
        let (sender, receiver) = mpsc::channel();
        for reply in replies {
            sender.send(reply).unwrap();
        }
        let events = Mutex::new(Vec::new());
        let report = |event| events.lock().unwrap().push(event);
        let spec = OperationSpec {
            kind: OperationKind::Find,
            sources: Sources::Find(FindQuery {
                dir: dir.to_path_buf(),
                pattern: Filter::parse(pattern).unwrap().unwrap(),
                show_hidden,
                max_results,
            }),
            dest: None,
            output: None,
        };

        run_operation(&spec, None, &NestedArchives::default(), &receiver, &report);

        let mut events = events.into_inner().unwrap();
        let Some(OperationEvent::Finished(outcome)) = events.pop() else {
            panic!("the find did not finish");
        };
        (outcome, events)
    }
}
