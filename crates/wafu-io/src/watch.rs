use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use notify::event::{AccessKind, AccessMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use wafu_core::{IoError, IoEvent, Side};

use crate::runtime::OnEvent;

const DEBOUNCE: Duration = Duration::from_millis(200);

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
        // The directory's own watch goes quiet when the directory is removed
        // or renamed (Windows reports nothing at all), so the parent is
        // watched for that one entry as well. Failing to is not worth a report.
        if let Some(parent) = path.parent()
            && let Ok(watcher) = watch_entry(parent, &path, changes)
        {
            self.sides[side.index()].push(watcher);
        }
    }

    pub(crate) fn unwatch(&mut self, side: Side) {
        self.sides[side.index()].clear();
    }
}

// Editors often replace the file rather than write into it, so its directory
// is watched and the file is read again once the changes go quiet.
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
        let counts = match result {
            Ok(event) => is_change(event.kind),
            Err(_) => true,
        };
        if counts {
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
        if let Ok(event) = result
            && is_change(event.kind)
            && event
                .paths
                .iter()
                .any(|path| path.file_name().map(ToOwned::to_owned) == name)
        {
            let _ = changes.send(());
        }
    })?;
    watcher.watch(dir, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

// Opening or reading the directory is reported as an access on Linux, and the
// listing itself does that, so accesses must not count as changes.
fn is_change(kind: EventKind) -> bool {
    match kind {
        EventKind::Access(access) => access == AccessKind::Close(AccessMode::Write),
        _ => true,
    }
}

// Calls `on_quiet` once per burst of changes, after they stop for a moment.
// The thread ends when the senders are dropped.
fn spawn_debouncer(changes: mpsc::Receiver<()>, on_quiet: impl Fn() + Send + 'static) {
    thread::Builder::new()
        .name("wafu-io-watch".to_owned())
        .spawn(move || {
            while changes.recv().is_ok() {
                loop {
                    match changes.recv_timeout(DEBOUNCE) {
                        Ok(()) => {}
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                on_quiet();
            }
        })
        .expect("the watch thread should start");
}
