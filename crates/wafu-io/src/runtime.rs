use std::collections::HashSet;
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use wafu_core::{Effect, IoError, IoEvent, Location, RequestId, Side};

use crate::read_dir::read_dir;
use crate::watch::Watcher;

pub(crate) type OnEvent = Arc<dyn Fn(IoEvent) + Send + Sync>;

/// Executes effects on behalf of the application and reports the results
/// through the callback given to [`Runtime::new`]. The callback is invoked on
/// worker threads.
pub struct Runtime {
    jobs: Sender<Job>,
    pending: Arc<Mutex<HashSet<RequestId>>>,
    watcher: Mutex<Watcher>,
    on_event: OnEvent,
}

#[derive(Debug)]
enum Job {
    ReadDir {
        side: Side,
        location: Location,
        request: RequestId,
    },
}

impl Runtime {
    pub fn new(on_event: impl Fn(IoEvent) + Send + Sync + 'static) -> Self {
        let on_event: OnEvent = Arc::new(on_event);
        let (jobs, receiver) = mpsc::channel::<Job>();
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let worker_pending = Arc::clone(&pending);
        let worker_event = Arc::clone(&on_event);
        thread::Builder::new()
            .name("wafu-io-reader".to_owned())
            .spawn(move || {
                for job in receiver {
                    let Job::ReadDir {
                        side,
                        location,
                        request,
                    } = job;
                    if !lock(&worker_pending).contains(&request) {
                        continue;
                    }
                    let result =
                        read_dir(&location.base).map_err(|error| IoError::new(error.to_string()));
                    if !lock(&worker_pending).remove(&request) {
                        continue;
                    }
                    worker_event(IoEvent::DirLoaded {
                        side,
                        request,
                        result,
                    });
                }
            })
            .expect("the reader thread should start");
        Self {
            jobs,
            pending,
            watcher: Mutex::new(Watcher::new()),
            on_event,
        }
    }

    /// Runs an effect. Effects that are not for the I/O layer are ignored.
    pub fn run(&self, effect: Effect) {
        match effect {
            Effect::ReadDir {
                side,
                location,
                request,
            } => {
                lock(&self.pending).insert(request);
                let _ = self.jobs.send(Job::ReadDir {
                    side,
                    location,
                    request,
                });
            }
            Effect::CancelRead { request } => {
                lock(&self.pending).remove(&request);
            }
            Effect::Watch { side, path } => {
                self.watcher
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .watch(side, path, &self.on_event);
            }
            Effect::SetTitle(_) => {}
        }
    }
}

fn lock(pending: &Mutex<HashSet<RequestId>>) -> std::sync::MutexGuard<'_, HashSet<RequestId>> {
    pending.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::fs::File;
    use std::path::Path;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use tempfile::TempDir;

    use super::*;

    const TIMEOUT: Duration = Duration::from_secs(5);
    const QUIET: Duration = Duration::from_millis(600);

    #[test]
    fn read_dir_reports_entries() {
        let dir = TempDir::new().unwrap();
        File::create(dir.path().join("a")).unwrap();
        let (runtime, events) = runtime();

        runtime.run(read(dir.path(), 1));

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::DirLoaded {
            side,
            request,
            result,
        } = event
        else {
            panic!("expected DirLoaded, got {event:?}");
        };
        assert_eq!(side, Side::Left);
        assert_eq!(request, RequestId(1));
        let entries = result.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "a");
    }

    #[test]
    fn read_dir_reports_errors() {
        let dir = TempDir::new().unwrap();
        let (runtime, events) = runtime();

        runtime.run(read(&dir.path().join("missing"), 1));

        let IoEvent::DirLoaded { result, .. } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected DirLoaded");
        };
        assert!(result.is_err());
    }

    #[test]
    fn cancelled_request_is_not_reported() {
        let dir = TempDir::new().unwrap();
        let (runtime, events) = runtime();

        runtime.run(read(dir.path(), 1));
        runtime.run(Effect::CancelRead {
            request: RequestId(1),
        });
        runtime.run(read(dir.path(), 2));

        let IoEvent::DirLoaded { request, .. } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected DirLoaded");
        };
        assert_eq!(request, RequestId(2));
        assert!(events.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn change_in_a_watched_directory_is_reported_once_per_burst() {
        let dir = TempDir::new().unwrap();
        let (runtime, events) = runtime();
        runtime.run(watch(dir.path()));
        // Give the watch time to become active before changing anything.
        std::thread::sleep(Duration::from_millis(200));

        File::create(dir.path().join("a")).unwrap();
        File::create(dir.path().join("b")).unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            IoEvent::LocationChanged {
                side: Side::Left,
                path: dir.path().to_path_buf(),
            }
        );
        assert!(events.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn replacing_a_watch_stops_reports_for_the_old_path() {
        let old = TempDir::new().unwrap();
        let new = TempDir::new().unwrap();
        let (runtime, events) = runtime();
        runtime.run(watch(old.path()));
        runtime.run(watch(new.path()));
        std::thread::sleep(Duration::from_millis(200));

        File::create(old.path().join("a")).unwrap();
        assert!(events.recv_timeout(QUIET).is_err());

        File::create(new.path().join("b")).unwrap();
        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            IoEvent::LocationChanged {
                side: Side::Left,
                path: new.path().to_path_buf(),
            }
        );
    }

    #[test]
    fn reading_the_watched_directory_is_not_a_change() {
        let dir = TempDir::new().unwrap();
        File::create(dir.path().join("a")).unwrap();
        let (runtime, events) = runtime();
        runtime.run(watch(dir.path()));
        std::thread::sleep(Duration::from_millis(200));

        runtime.run(read(dir.path(), 1));
        let IoEvent::DirLoaded { .. } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected DirLoaded");
        };

        assert!(events.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn watching_a_missing_path_fails() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("missing");
        let (runtime, events) = runtime();

        runtime.run(watch(&missing));

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::WatchFailed { side, path, .. } = event else {
            panic!("expected WatchFailed, got {event:?}");
        };
        assert_eq!(side, Side::Left);
        assert_eq!(path, missing);
    }

    fn runtime() -> (Runtime, Receiver<IoEvent>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        let runtime = Runtime::new(move |event| {
            let _ = sender
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .send(event);
        });
        (runtime, receiver)
    }

    fn read(path: &Path, request: u64) -> Effect {
        Effect::ReadDir {
            side: Side::Left,
            location: Location::new(path.to_path_buf()),
            request: RequestId(request),
        }
    }

    fn watch(path: &Path) -> Effect {
        Effect::Watch {
            side: Side::Left,
            path: path.to_path_buf(),
        }
    }
}
