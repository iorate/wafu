use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
use icu_normalizer::ComposingNormalizer;
use notify::event::{AccessKind, AccessMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use wafu_core::{IoError, IoEvent, Side};

use crate::runtime::OnEvent;

const DEBOUNCE: Duration = Duration::from_millis(200);
const MAX_WAIT: Duration = Duration::from_secs(1);

// Each pane has its own watchers and debounce thread so that the callback knows
// which pane and path it reports for. Dropping the watchers ends the thread.
pub(crate) struct Watcher {
    sides: [Vec<RecommendedWatcher>; 2],
}

impl Watcher {
    pub(crate) fn new() -> Self {
        Self {
            sides: [Vec::new(), Vec::new()],
        }
    }

    pub(crate) fn watch(&mut self, side: Side, path: PathBuf, on_event: &OnEvent) {
        self.unwatch(side);
        let (changes, receiver) = mpsc::channel::<()>();
        let reported = path.clone();
        let report = Arc::clone(on_event);
        spawn_debouncer(receiver, move || {
            report(IoEvent::LocationChanged {
                side,
                path: reported.clone(),
            });
        });
        // A file (an archive being shown) has no contents to watch; its
        // changes come through its directory.
        let is_dir = match fs::metadata(&path) {
            Ok(meta) => meta.is_dir(),
            Err(error) => {
                on_event(IoEvent::WatchFailed {
                    side,
                    path,
                    error: IoError::new(error.to_string()),
                });
                return;
            }
        };
        if is_dir {
            match watch_contents(&path, changes.clone()) {
                Ok(watcher) => self.sides[side.index()].push(watcher),
                Err(error) => {
                    on_event(IoEvent::WatchFailed {
                        side,
                        path,
                        error: IoError::new(error.to_string()),
                    });
                    return;
                }
            }
        }
        // The directory's own watch goes quiet when the directory is removed
        // or renamed (Windows reports nothing at all), so the parent is
        // watched for that one entry as well. For a directory, failing to is
        // not worth a report; for a file it is the only watch there is.
        match path
            .parent()
            .map(|parent| watch_entry(parent, &path, changes))
        {
            Some(Ok(watcher)) => self.sides[side.index()].push(watcher),
            Some(Err(error)) if !is_dir => on_event(IoEvent::WatchFailed {
                side,
                path,
                error: IoError::new(error.to_string()),
            }),
            _ => {}
        }
    }

    pub(crate) fn unwatch(&mut self, side: Side) {
        self.sides[side.index()].clear();
    }
}

// Editors often replace the file rather than write into it, so its directory
// is watched and the file is read again after each burst of changes.
pub(crate) fn watch_config(path: &Path, on_event: &OnEvent) -> notify::Result<RecommendedWatcher> {
    let parent = path
        .parent()
        .ok_or_else(|| notify::Error::generic("the configuration file has no parent directory"))?;
    let (changes, receiver) = mpsc::channel::<()>();
    let report = Arc::clone(on_event);
    let read_path = path.to_path_buf();
    spawn_debouncer(receiver, move || {
        let result =
            fs::read_to_string(&read_path).map_err(|error| IoError::new(error.to_string()));
        report(IoEvent::ConfigLoaded { result });
    });
    watch_entry(parent, path, changes)
}

// Reports any change inside the directory. An error from the watcher counts
// as a change too, since a failed read will sort out what happened.
fn watch_contents(path: &Path, changes: Sender<()>) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if counts_as_change(&result, |_| true) {
            let _ = changes.send(());
        }
    })?;
    watcher.watch(path, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

// Reports changes to one entry of a directory, such as its removal. The
// entry is matched by name, since the paths reported may spell the directory
// differently (macOS reports `/private/var` for `/var`).
fn watch_entry(
    dir: &Path,
    entry: &Path,
    changes: Sender<()>,
) -> notify::Result<RecommendedWatcher> {
    let name = entry.file_name().map(ToOwned::to_owned);
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if counts_as_change(&result, |event| names_entry(event, name.as_deref())) {
            let _ = changes.send(());
        }
    })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

fn counts_as_change(result: &notify::Result<Event>, concerns: impl FnOnce(&Event) -> bool) -> bool {
    match result {
        Ok(event) => event.need_rescan() || (is_change(event.kind) && concerns(event)),
        Err(_) => true,
    }
}

fn names_entry(event: &Event, name: Option<&OsStr>) -> bool {
    event.paths.iter().any(|path| {
        path.file_name()
            .zip(name)
            .is_some_and(|(reported, watched)| same_name(reported, watched))
    })
}

#[cfg(target_os = "macos")]
fn same_name(reported: &OsStr, watched: &OsStr) -> bool {
    let fold = |name: &OsStr| {
        ComposingNormalizer::new_nfc()
            .normalize(&name.to_string_lossy())
            .to_lowercase()
    };
    fold(reported) == fold(watched)
}

#[cfg(windows)]
fn same_name(reported: &OsStr, watched: &OsStr) -> bool {
    reported.to_string_lossy().to_lowercase() == watched.to_string_lossy().to_lowercase()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn same_name(reported: &OsStr, watched: &OsStr) -> bool {
    reported == watched
}

// Opening or reading the directory is reported as an access on Linux, and the
// listing itself does that, so accesses must not count as changes.
fn is_change(kind: EventKind) -> bool {
    match kind {
        EventKind::Access(access) => access == AccessKind::Close(AccessMode::Write),
        _ => true,
    }
}

// Calls `on_burst` once per burst of changes, after they stop for a moment or
// after a while even if they do not, so that a directory written to all the
// time is still refreshed. The thread ends when the senders are dropped.
fn spawn_debouncer(changes: mpsc::Receiver<()>, on_burst: impl Fn() + Send + 'static) {
    thread::Builder::new()
        .name("wafu-io-watch".to_owned())
        .spawn(move || {
            while changes.recv().is_ok() {
                let first = Instant::now();
                let mut last = first;
                loop {
                    let deadline = (last + DEBOUNCE).min(first + MAX_WAIT);
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    match changes.recv_timeout(deadline - now) {
                        Ok(()) => last = Instant::now(),
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                on_burst();
            }
        })
        .expect("the watch thread should start");
}

#[cfg(test)]
mod tests {
    use notify::event::{CreateKind, Flag, RemoveKind};

    use super::*;

    #[test]
    fn an_error_counts_as_a_change() {
        let error = Err(notify::Error::generic("queue overflow"));
        assert!(counts_as_change(&error, |_| true));
        assert!(counts_as_change(&error, |event| names_entry(
            event,
            Some(OsStr::new("Archive.zip"))
        )));
    }

    #[test]
    fn a_rescan_without_paths_counts_as_a_change() {
        let rescan = Ok(Event::new(EventKind::Other).set_flag(Flag::Rescan));
        assert!(counts_as_change(&rescan, |_| true));
        assert!(counts_as_change(&rescan, |event| names_entry(
            event,
            Some(OsStr::new("Archive.zip"))
        )));
    }

    #[test]
    fn an_entry_change_counts_only_for_the_watched_name() {
        let dir = Path::new("dir");
        let removed =
            Ok(Event::new(EventKind::Remove(RemoveKind::Any)).add_path(dir.join("Archive.zip")));
        let created =
            Ok(Event::new(EventKind::Create(CreateKind::Any)).add_path(dir.join("Other.zip")));
        let watched = Some(OsStr::new("Archive.zip"));
        assert!(counts_as_change(&removed, |event| names_entry(
            event, watched
        )));
        assert!(!counts_as_change(&created, |event| names_entry(
            event, watched
        )));
    }

    #[test]
    fn an_access_does_not_count_as_a_change() {
        let opened = Ok(
            Event::new(EventKind::Access(AccessKind::Open(AccessMode::Any)))
                .add_path(PathBuf::from("dir").join("Archive.zip")),
        );
        assert!(!counts_as_change(&opened, |_| true));
        assert!(!counts_as_change(&opened, |event| names_entry(
            event,
            Some(OsStr::new("Archive.zip"))
        )));
    }

    #[test]
    fn same_name_matches_an_identical_name() {
        assert!(same_name(
            OsStr::new("Archive.zip"),
            OsStr::new("Archive.zip")
        ));
        assert!(!same_name(
            OsStr::new("Archive.zip"),
            OsStr::new("Other.zip")
        ));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn same_name_ignores_case() {
        assert!(same_name(
            OsStr::new("Archive.zip"),
            OsStr::new("archive.ZIP")
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn same_name_ignores_normalization() {
        assert!(same_name(
            OsStr::new("\u{304b}\u{3099}"),
            OsStr::new("\u{304c}")
        ));
        assert!(same_name(
            OsStr::new("\u{30ac}.ZIP"),
            OsStr::new("\u{30ab}\u{3099}.zip")
        ));
    }

    #[cfg(not(any(windows, target_os = "macos")))]
    #[test]
    fn same_name_compares_bytes() {
        assert!(!same_name(
            OsStr::new("Archive.zip"),
            OsStr::new("archive.ZIP")
        ));
        assert!(!same_name(
            OsStr::new("\u{304b}\u{3099}"),
            OsStr::new("\u{304c}")
        ));
    }

    #[test]
    fn changes_that_never_stop_are_still_reported() {
        let (changes, receiver) = mpsc::channel();
        let (reports, reported) = mpsc::channel();
        spawn_debouncer(receiver, move || {
            let _ = reports.send(());
        });

        let start = Instant::now();
        while reported.try_recv().is_err() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "no report while changes kept coming"
            );
            changes.send(()).unwrap();
            thread::sleep(Duration::from_millis(20));
        }
    }
}
