use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::Duration;

use notify::event::{AccessKind, AccessMode};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use wafu_core::{IoError, IoEvent, Side};

use crate::runtime::OnEvent;

const DEBOUNCE: Duration = Duration::from_millis(200);

// Each pane has its own watcher and debounce thread so that the callback knows
// which pane and path it reports for. Dropping the watcher ends the thread.
pub(crate) struct Watcher {
    sides: [Option<RecommendedWatcher>; 2],
}

impl Watcher {
    pub(crate) fn new() -> Self {
        Self {
            sides: [None, None],
        }
    }

    pub(crate) fn watch(&mut self, side: Side, path: PathBuf, on_event: &OnEvent) {
        self.unwatch(side);
        let (changes, receiver) = mpsc::channel::<()>();
        spawn_debouncer(side, path.clone(), Arc::clone(on_event), receiver);
        // An error from the watcher counts as a change too: on Windows that is
        // how the removal of the watched directory itself shows up.
        let result = notify::recommended_watcher(move |result: notify::Result<Event>| {
            let counts = match result {
                Ok(event) => is_change(event.kind),
                Err(_) => true,
            };
            if counts {
                let _ = changes.send(());
            }
        })
        .and_then(|mut watcher| {
            watcher
                .watch(&path, RecursiveMode::NonRecursive)
                .map(|()| watcher)
        });
        match result {
            Ok(watcher) => self.sides[side.index()] = Some(watcher),
            Err(error) => on_event(IoEvent::WatchFailed {
                side,
                path,
                error: IoError::new(error.to_string()),
            }),
        }
    }

    pub(crate) fn unwatch(&mut self, side: Side) {
        self.sides[side.index()] = None;
    }
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
