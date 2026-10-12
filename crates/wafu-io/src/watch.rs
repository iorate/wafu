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
        spawn_debouncer(side, path.clone(), Arc::clone(on_event), receiver);
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
            && let Ok(watcher) = watch_entry(parent, path.clone(), changes)
        {
            self.sides[side.index()].push(watcher);
        }
    }

    pub(crate) fn unwatch(&mut self, side: Side) {
        self.sides[side.index()].clear();
    }
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

// Reports changes to one entry of a directory, such as its removal.
fn watch_entry(
    dir: &Path,
    entry: PathBuf,
    changes: Sender<()>,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if let Ok(event) = result
            && is_change(event.kind)
            && event.paths.contains(&entry)
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

fn spawn_debouncer(side: Side, path: PathBuf, on_event: OnEvent, changes: mpsc::Receiver<()>) {
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
                on_event(IoEvent::LocationChanged {
                    side,
                    path: path.clone(),
                });
            }
        })
        .expect("the watch thread should start");
}
