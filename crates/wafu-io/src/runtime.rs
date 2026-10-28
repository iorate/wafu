use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;

use notify::RecommendedWatcher;
use wafu_core::{
    ArchivePath, Change, Directory, Effect, IoError, IoEvent, Location, PreviewSource, Reply,
    RequestId, Side, SpawnKind,
};

use crate::archive::{ArchiveEntry, dir_entry, list_dir, open_archive};
use crate::drives::drives;
use crate::fs_ops::{create_file, mkdir, open_with_os, rename, set_clipboard};
use crate::operation::run_operation;
use crate::preview::{load_image, load_preview};
use crate::read_dir::{own_entry, read_dir_with};
use crate::resolve::resolve;
use crate::spawn::{spawn, spawn_new_window};
use crate::state::{write_handoff, write_state};
use crate::watch::{Watcher, watch_config};

pub(crate) type OnEvent = Arc<dyn Fn(IoEvent) + Send + Sync>;

/// Where the runtime writes the state, and the configuration path to pass on
/// to a new instance when it was given on the command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub state: PathBuf,
    pub config_override: Option<PathBuf>,
}

/// Executes effects on behalf of the application and reports the results
/// through the callback given to [`Runtime::new`]. The callback is invoked on
/// worker threads, or on the caller's thread for effects that finish at once.
pub struct Runtime {
    paths: Paths,
    jobs: Sender<Job>,
    pending: Arc<Mutex<HashSet<RequestId>>>,
    watcher: Mutex<Watcher>,
    config_watcher: Mutex<Option<RecommendedWatcher>>,
    // The running operation takes its replies from here.
    operation: Mutex<Option<Sender<Reply>>>,
    clipboard: Mutex<Option<arboard::Clipboard>>,
    on_event: OnEvent,
}

#[derive(Debug)]
enum Job {
    ReadDir {
        side: Side,
        location: Location,
        request: RequestId,
    },
    Resolve {
        side: Side,
        path: PathBuf,
        climb: bool,
        request: RequestId,
    },
    LoadPreview {
        source: PreviewSource,
        request: RequestId,
    },
    LoadImage {
        source: PreviewSource,
        request: RequestId,
    },
}

impl Runtime {
    pub fn new(paths: Paths, on_event: impl Fn(IoEvent) + Send + Sync + 'static) -> Self {
        let on_event: OnEvent = Arc::new(on_event);
        let (jobs, receiver) = mpsc::channel::<Job>();
        let pending = Arc::new(Mutex::new(HashSet::new()));
        let worker_pending = Arc::clone(&pending);
        let worker_event = Arc::clone(&on_event);
        thread::Builder::new()
            .name("wafu-io-reader".to_owned())
            .spawn(move || {
                let mut cache: Option<CachedArchive> = None;
                for job in receiver {
                    let request = match &job {
                        Job::ReadDir { request, .. }
                        | Job::Resolve { request, .. }
                        | Job::LoadPreview { request, .. }
                        | Job::LoadImage { request, .. } => *request,
                    };
                    if !lock(&worker_pending).contains(&request) {
                        continue;
                    }
                    let event = match job {
                        Job::ReadDir { side, location, .. } => {
                            let cancelled = || !lock(&worker_pending).contains(&request);
                            let result = match &location.archive {
                                None => read_dir_with(&location.base, cancelled),
                                Some(archive) => {
                                    list_archive(&mut cache, &location.base, archive, cancelled)
                                }
                            }
                            .map_err(|error| IoError::new(error.to_string()));
                            IoEvent::DirLoaded {
                                side,
                                request,
                                result,
                            }
                        }
                        Job::Resolve {
                            side, path, climb, ..
                        } => IoEvent::Resolved {
                            side,
                            request,
                            result: resolve(&path, climb),
                        },
                        Job::LoadPreview { source, .. } => IoEvent::PreviewLoaded {
                            request,
                            result: load_preview(&source),
                        },
                        Job::LoadImage { source, .. } => IoEvent::ImageLoaded {
                            request,
                            result: load_image(&source),
                        },
                    };
                    if !lock(&worker_pending).remove(&request) {
                        continue;
                    }
                    worker_event(event);
                }
            })
            .expect("the reader thread should start");
        Self {
            paths,
            jobs,
            pending,
            watcher: Mutex::new(Watcher::new()),
            config_watcher: Mutex::new(None),
            operation: Mutex::new(None),
            clipboard: Mutex::new(None),
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
            Effect::Resolve {
                side,
                path,
                climb,
                request,
            } => {
                // The old watch might keep a vanished directory half alive on
                // Windows; the application watches again once it knows where it is.
                self.watcher
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .unwatch(side);
                lock(&self.pending).insert(request);
                let _ = self.jobs.send(Job::Resolve {
                    side,
                    path,
                    climb,
                    request,
                });
            }
            Effect::CancelRead { request } => {
                lock(&self.pending).remove(&request);
            }
            Effect::LoadPreview { source, request } => {
                lock(&self.pending).insert(request);
                let _ = self.jobs.send(Job::LoadPreview { source, request });
            }
            Effect::LoadImage { source, request } => {
                lock(&self.pending).insert(request);
                let _ = self.jobs.send(Job::LoadImage { source, request });
            }
            Effect::Watch { side, path } => {
                self.watcher
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .watch(side, path, &self.on_event);
            }
            Effect::ListDrives { purpose } => (self.on_event)(IoEvent::Drives {
                purpose,
                letters: drives(),
            }),
            Effect::WatchConfig(path) => {
                let watcher = match watch_config(&path, &self.on_event) {
                    Ok(watcher) => Some(watcher),
                    Err(error) => {
                        (self.on_event)(IoEvent::ConfigWatchFailed {
                            error: IoError::new(error.to_string()),
                        });
                        None
                    }
                };
                *self
                    .config_watcher
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = watcher;
            }
            Effect::Spawn { program, args, cwd } => {
                let error = spawn(&program, &args, &cwd)
                    .err()
                    .map(|error| IoError::new(error.to_string()));
                (self.on_event)(IoEvent::Spawned {
                    kind: SpawnKind::External { program },
                    error,
                });
            }
            // A failure here has nowhere to go: the application is quitting.
            Effect::SaveState(state) => {
                let _ = write_state(&self.paths.state, &state);
            }
            Effect::SpawnNewWindow(state) => {
                let error = write_handoff(&state)
                    .and_then(|handoff| {
                        spawn_new_window(&handoff, self.paths.config_override.as_deref())
                    })
                    .err()
                    .map(|error| IoError::new(error.to_string()));
                (self.on_event)(IoEvent::Spawned {
                    kind: SpawnKind::NewWindow,
                    error,
                });
            }
            effect => self.run_file_effect(effect),
        }
    }

    fn run_file_effect(&self, effect: Effect) {
        match effect {
            Effect::StartOperation(spec) => {
                let (sender, replies) = mpsc::channel();
                *self
                    .operation
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(sender);
                let on_event = Arc::clone(&self.on_event);
                thread::Builder::new()
                    .name("wafu-io-operation".to_owned())
                    .spawn(move || {
                        run_operation(&spec, &replies, &|event| {
                            on_event(IoEvent::Operation(event));
                        });
                    })
                    .expect("the operation thread should start");
            }
            Effect::OperationReply(reply) => {
                if let Some(sender) = self
                    .operation
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                {
                    let _ = sender.send(reply);
                }
            }
            Effect::Rename { from, to } => {
                let error = io_error(rename(&from, &to));
                (self.on_event)(IoEvent::Changed {
                    change: Change::Renamed { from, to },
                    error,
                });
            }
            Effect::Mkdir(path) => {
                let error = io_error(mkdir(&path));
                (self.on_event)(IoEvent::Changed {
                    change: Change::DirCreated(path),
                    error,
                });
            }
            Effect::CreateFile(path) => {
                let error = io_error(create_file(&path));
                (self.on_event)(IoEvent::Changed {
                    change: Change::FileCreated(path),
                    error,
                });
            }
            Effect::Clipboard(text) => {
                let error = set_clipboard(
                    &mut self
                        .clipboard
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner),
                    &text,
                )
                .err()
                .map(|error| IoError::new(error.to_string()));
                (self.on_event)(IoEvent::ClipboardSet { text, error });
            }
            Effect::OpenWithOs(path) => {
                let error = io_error(open_with_os(&path));
                (self.on_event)(IoEvent::Opened { path, error });
            }
            _ => {}
        }
    }
}

// The table of contents of the archive read last, so that moving between
// its directories does not read it again. A changed archive misses the cache.
struct CachedArchive {
    path: PathBuf,
    len: u64,
    modified: Option<std::time::SystemTime>,
    entries: Vec<ArchiveEntry>,
}

fn list_archive(
    cache: &mut Option<CachedArchive>,
    path: &Path,
    archive: &ArchivePath,
    cancelled: impl Fn() -> bool,
) -> io::Result<Directory> {
    let meta = fs::metadata(path)?;
    let (len, modified) = (meta.len(), meta.modified().ok());
    let fresh = cache.as_ref().is_some_and(|cached| {
        cached.path == path && cached.len == len && cached.modified == modified
    });
    if !fresh {
        let reader = open_archive(path, archive.format).map_err(io::Error::other)?;
        *cache = Some(CachedArchive {
            path: path.to_path_buf(),
            len,
            modified,
            entries: reader.entries().to_vec(),
        });
    }
    let entries = &cache.as_ref().expect("the cache was just filled").entries;
    let own = dir_entry(entries, &archive.inner).or_else(|| own_entry(path));
    let entries = list_dir(entries, &archive.inner, cancelled)?;
    Ok(Directory { own, entries })
}

fn io_error(result: std::io::Result<()>) -> Option<IoError> {
    result.err().map(|error| IoError::new(error.to_string()))
}

fn lock(pending: &Mutex<HashSet<RequestId>>) -> std::sync::MutexGuard<'_, HashSet<RequestId>> {
    pending.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::fs::{self, File};
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
        let entries = result.unwrap().entries;
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

        with_worker_held(&runtime, |pending, jobs| {
            pending.insert(RequestId(1));
            jobs.send(read_job(dir.path(), 1)).unwrap();
            pending.remove(&RequestId(1));
            pending.insert(RequestId(2));
            jobs.send(read_job(dir.path(), 2)).unwrap();
        });

        let IoEvent::DirLoaded { request, .. } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected DirLoaded");
        };
        assert_eq!(request, RequestId(2));
        assert!(events.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn resolve_reports_the_nearest_directory() {
        let dir = TempDir::new().unwrap();
        let (runtime, events) = runtime();

        runtime.run(Effect::Resolve {
            side: Side::Right,
            path: dir.path().join("gone"),
            climb: true,
            request: RequestId(1),
        });

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            IoEvent::Resolved {
                side: Side::Right,
                request: RequestId(1),
                result: Some(wafu_core::Resolved {
                    dir: dir.path().to_path_buf(),
                    file: None,
                }),
            }
        );
    }

    #[test]
    fn cancelled_resolve_is_not_reported() {
        let dir = TempDir::new().unwrap();
        let (runtime, events) = runtime();

        with_worker_held(&runtime, |pending, jobs| {
            pending.insert(RequestId(1));
            jobs.send(Job::Resolve {
                side: Side::Left,
                path: dir.path().to_path_buf(),
                climb: true,
                request: RequestId(1),
            })
            .unwrap();
            pending.remove(&RequestId(1));
        });

        assert!(events.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn a_preview_is_read_on_the_worker_and_can_be_cancelled() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.txt");
        fs::write(&path, b"hello\n").unwrap();
        let (runtime, events) = runtime();
        let source = PreviewSource {
            path,
            entry: None,
            password: None,
        };

        with_worker_held(&runtime, |pending, jobs| {
            pending.insert(RequestId(1));
            jobs.send(Job::LoadPreview {
                source: source.clone(),
                request: RequestId(1),
            })
            .unwrap();
            pending.remove(&RequestId(1));
            pending.insert(RequestId(2));
            jobs.send(Job::LoadPreview {
                source,
                request: RequestId(2),
            })
            .unwrap();
        });

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::PreviewLoaded { request, result } = event else {
            panic!("expected PreviewLoaded, got {event:?}");
        };
        assert_eq!(request, RequestId(2));
        assert_eq!(result.unwrap().lines[0].text, "hello");
        assert!(events.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[test]
    fn an_image_that_cannot_be_decoded_is_reported() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("a.png");
        fs::write(&path, b"nope").unwrap();
        let (runtime, events) = runtime();

        runtime.run(Effect::LoadImage {
            source: PreviewSource {
                path,
                entry: None,
                password: None,
            },
            request: RequestId(1),
        });

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::ImageLoaded { request, result } = event else {
            panic!("expected ImageLoaded, got {event:?}");
        };
        assert_eq!(request, RequestId(1));
        assert!(matches!(result, Err(wafu_core::PreviewError::Failed(_))));
    }

    #[test]
    fn list_drives_replies_at_once() {
        let (runtime, events) = runtime();

        runtime.run(Effect::ListDrives {
            purpose: wafu_core::DrivePurpose::Show,
        });

        let IoEvent::Drives { purpose, letters } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected Drives");
        };
        assert_eq!(purpose, wafu_core::DrivePurpose::Show);
        assert_eq!(letters.is_empty(), !cfg!(windows));
    }

    #[test]
    fn removing_the_watched_directory_is_reported() {
        let parent = TempDir::new().unwrap();
        let dir = parent.path().join("watched");
        fs::create_dir(&dir).unwrap();
        let (runtime, events) = runtime();
        runtime.run(watch(&dir));
        settle(&events);

        fs::remove_dir(&dir).unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            IoEvent::LocationChanged {
                side: Side::Left,
                path: dir,
            }
        );
    }

    #[test]
    fn change_in_a_watched_directory_is_reported_once_per_burst() {
        let dir = TempDir::new().unwrap();
        let (runtime, events) = runtime();
        runtime.run(watch(dir.path()));
        settle(&events);

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
        settle(&events);

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
        settle(&events);

        runtime.run(read(dir.path(), 1));
        let IoEvent::DirLoaded { .. } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected DirLoaded");
        };

        assert!(events.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn changes_to_a_watched_file_are_reported() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a.zip");
        fs::write(&file, b"v1").unwrap();
        let (runtime, events) = runtime();
        runtime.run(watch(&file));
        settle(&events);

        fs::write(&file, b"v2").unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            IoEvent::LocationChanged {
                side: Side::Left,
                path: file.clone(),
            }
        );
        settle(&events);

        File::create(dir.path().join("other")).unwrap();
        assert!(events.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn read_dir_inside_an_archive_lists_its_entries_and_reuses_the_table() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.zip");
        {
            let mut writer = crate::archive::create_archive(
                File::create(&archive).unwrap(),
                wafu_core::ArchiveFormat::Zip,
                None,
            );
            writer.start_file("d/f.txt", 1, None, None).unwrap();
            std::io::Write::write_all(&mut writer, b"f").unwrap();
            writer.finish().unwrap();
        }
        let (runtime, events) = runtime();
        let root = Location::archive_root(archive.clone(), wafu_core::ArchiveFormat::Zip);

        runtime.run(Effect::ReadDir {
            side: Side::Left,
            location: root.clone(),
            request: RequestId(1),
        });
        runtime.run(Effect::ReadDir {
            side: Side::Left,
            location: root.child("d"),
            request: RequestId(2),
        });

        let IoEvent::DirLoaded { result, .. } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected DirLoaded");
        };
        let directory = result.unwrap();
        assert_eq!(directory.own.unwrap().name, "a.zip");
        let entries = directory.entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "d");
        assert_eq!(entries[0].kind, wafu_core::EntryKind::Dir);
        let IoEvent::DirLoaded { result, .. } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected DirLoaded");
        };
        let directory = result.unwrap();
        let own = directory.own.unwrap();
        assert_eq!(own.name, "d");
        assert_eq!(own.kind, wafu_core::EntryKind::Dir);
        let entries = directory.entries;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "f.txt");
        assert_eq!(entries[0].size, 1);
    }

    #[test]
    fn config_changes_are_reported_with_the_new_text() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "a = 1\n").unwrap();
        let (runtime, events) = runtime();
        runtime.run(Effect::WatchConfig(path.clone()));
        settle(&events);

        // Written the way editors do it: a new file moved over the old one.
        let temp = dir.path().join("config.toml.tmp");
        fs::write(&temp, "a = 2\n").unwrap();
        fs::rename(&temp, &path).unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            IoEvent::ConfigLoaded {
                result: Ok("a = 2\n".to_owned()),
            }
        );
        assert!(events.recv_timeout(QUIET).is_err());

        fs::write(dir.path().join("other"), "x").unwrap();
        assert!(events.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn save_state_writes_the_state_file() {
        let dir = TempDir::new().unwrap();
        let (sender, _events) = mpsc::channel();
        let sender = Mutex::new(sender);
        let state_path = dir.path().join("sub").join("state.toml");
        let runtime = Runtime::new(
            Paths {
                state: state_path.clone(),
                config_override: None,
            },
            move |event| {
                let _ = sender
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .send(event);
            },
        );
        let state = wafu_core::StateFile {
            log_rows: Some(3),
            ..wafu_core::StateFile::default()
        };

        runtime.run(Effect::SaveState(state.clone()));

        assert_eq!(crate::state::read_state(&state_path), Ok(Some(state)));
    }

    #[test]
    fn a_spawn_that_fails_is_reported() {
        let dir = TempDir::new().unwrap();
        let (runtime, events) = runtime();

        runtime.run(Effect::Spawn {
            program: "wafu-no-such-program".to_owned(),
            args: Vec::new(),
            cwd: dir.path().to_path_buf(),
        });

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::Spawned { kind, error } = event else {
            panic!("expected Spawned, got {event:?}");
        };
        assert_eq!(
            kind,
            SpawnKind::External {
                program: "wafu-no-such-program".to_owned()
            }
        );
        assert!(error.is_some());
    }

    #[test]
    fn an_operation_runs_on_a_worker_and_takes_replies() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("src");
        let dst = dir.path().join("dst");
        fs::create_dir(&src).unwrap();
        fs::create_dir(&dst).unwrap();
        fs::write(src.join("a"), b"new").unwrap();
        fs::write(dst.join("a"), b"old").unwrap();
        let (runtime, events) = runtime();

        runtime.run(Effect::StartOperation(wafu_core::OperationSpec {
            kind: wafu_core::OperationKind::Copy,
            sources: wafu_core::Sources::Paths(vec![src.join("a")]),
            dest: Some(dst.clone()),
            output: None,
        }));

        let mut saw_conflict = false;
        loop {
            match events.recv_timeout(TIMEOUT).unwrap() {
                IoEvent::Operation(wafu_core::OperationEvent::Conflict { .. }) => {
                    saw_conflict = true;
                    runtime.run(Effect::OperationReply(Reply::Overwrite { all: false }));
                }
                IoEvent::Operation(wafu_core::OperationEvent::Finished(outcome)) => {
                    assert!(matches!(
                        outcome,
                        wafu_core::Outcome::Completed { done: 1, .. }
                    ));
                    break;
                }
                _ => {}
            }
        }
        assert!(saw_conflict);
        assert_eq!(fs::read(dst.join("a")).unwrap(), b"new");
    }

    #[test]
    fn small_changes_reply_at_once() {
        let dir = TempDir::new().unwrap();
        let (runtime, events) = runtime();

        runtime.run(Effect::Mkdir(dir.path().join("d")));
        assert_eq!(
            events.recv_timeout(TIMEOUT).unwrap(),
            IoEvent::Changed {
                change: Change::DirCreated(dir.path().join("d")),
                error: None,
            }
        );

        runtime.run(Effect::CreateFile(dir.path().join("d").join("f")));
        assert_eq!(
            events.recv_timeout(TIMEOUT).unwrap(),
            IoEvent::Changed {
                change: Change::FileCreated(dir.path().join("d").join("f")),
                error: None,
            }
        );

        runtime.run(Effect::Rename {
            from: dir.path().join("d").join("f"),
            to: dir.path().join("d"),
        });
        let IoEvent::Changed { error, .. } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected Changed");
        };
        assert_eq!(error, Some(IoError::new("already exists")));
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

    // A watch takes a moment to become active, and FSEvents on macOS may then
    // report changes made just before it started (such as creating the
    // directory itself), so wait until nothing has arrived for a while.
    fn settle(events: &Receiver<IoEvent>) {
        std::thread::sleep(Duration::from_millis(200));
        while events.recv_timeout(QUIET).is_ok() {}
    }

    fn runtime() -> (Runtime, Receiver<IoEvent>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        let paths = Paths {
            state: std::env::temp_dir().join("wafu-test-state.toml"),
            config_override: None,
        };
        let runtime = Runtime::new(paths, move |event| {
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

    fn read_job(path: &Path, request: u64) -> Job {
        Job::ReadDir {
            side: Side::Left,
            location: Location::new(path.to_path_buf()),
            request: RequestId(request),
        }
    }

    // Queues jobs and cancellations as `run` would, but with the worker held
    // off the pending set until all of them are in, so that a cancellation
    // cannot lose the race against a read that finishes at once.
    fn with_worker_held(
        runtime: &Runtime,
        queue: impl FnOnce(&mut HashSet<RequestId>, &Sender<Job>),
    ) {
        let mut pending = lock(&runtime.pending);
        queue(&mut pending, &runtime.jobs);
    }

    fn watch(path: &Path) -> Effect {
        Effect::Watch {
            side: Side::Left,
            path: path.to_path_buf(),
        }
    }
}
