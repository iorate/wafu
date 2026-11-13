use std::any::Any;
use std::cell::RefCell;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fmt::Display;
use std::fs;
use std::io;
#[cfg(unix)]
use std::io::IsTerminal;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;

use wafu_core::{
    ArchivePath, Change, Directory, Effect, IoError, IoEvent, Location, OperationEvent,
    OperationSpec, Outcome, PreviewError, PreviewSource, Reply, RequestId, Side, Sources,
    SpawnKind, SpawnName,
};

use crate::archive::{ArchiveEntry, dir_entry, list_dir, open_archive};
#[cfg(windows)]
use crate::color_scheme::{ColorSchemeWatch, watch_color_scheme};
use crate::drives::drives;
use crate::fs_ops::{create_file, mkdir, open_with_os, rename, set_clipboard};
use crate::operation::run_operation;
use crate::preview::{PreviewedArchive, load_image, load_preview};
use crate::read_dir::{own_entry, read_dir_with};
use crate::resolve::resolve;
#[cfg(unix)]
use crate::shell_env::load_shell_env;
use crate::spawn::{spawn, spawn_new_window, wait_in_background};
use crate::state::{write_handoff, write_state};
use crate::watch::{ConfigWatch, Watcher, watch_config};

const MAX_ABANDONED_PREVIEW_READERS: usize = 2;

/// Where the runtime writes the state, the configuration path to pass on to
/// a new instance when it was given on the command line, and the home
/// directory that `~` stands for in the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub state: PathBuf,
    pub config_override: Option<PathBuf>,
    pub home: PathBuf,
}

/// Executes effects on behalf of the application and reports the results
/// through the callback given to [`Runtime::new`]. The callback is invoked on
/// worker threads, or on the caller's thread for effects that finish at once.
pub struct Runtime {
    paths: Paths,
    left: Lane,
    right: Lane,
    preview: Lane,
    pending: Arc<Mutex<HashSet<RequestId>>>,
    watcher: Mutex<Watcher>,
    config_watcher: Mutex<Option<ConfigWatch>>,
    #[cfg(windows)]
    color_scheme_watch: Mutex<Option<ColorSchemeWatch>>,
    // The running operation takes its replies from here.
    operation: Mutex<Option<Sender<Reply>>>,
    owner_window: Mutex<Option<isize>>,
    clipboard: Mutex<Option<arboard::Clipboard>>,
    // The login shell's environment for the programs started, once read.
    shell_env: Arc<OnceLock<Vec<(OsString, OsString)>>>,
    on_event: OnEvent,
}

impl Runtime {
    pub fn new(paths: Paths, on_event: impl Fn(IoEvent) + Send + Sync + 'static) -> Self {
        let on_event: OnEvent = Arc::new(on_event);
        let pending = Arc::new(Mutex::new(HashSet::new()));
        Self {
            paths,
            left: Lane::new("left", None, &pending, &on_event),
            right: Lane::new("right", None, &pending, &on_event),
            preview: Lane::new(
                "preview",
                Some(MAX_ABANDONED_PREVIEW_READERS),
                &pending,
                &on_event,
            ),
            pending,
            watcher: Mutex::new(Watcher::new()),
            config_watcher: Mutex::new(None),
            #[cfg(windows)]
            color_scheme_watch: Mutex::new(None),
            operation: Mutex::new(None),
            owner_window: Mutex::new(None),
            clipboard: Mutex::new(None),
            shell_env: Arc::new(OnceLock::new()),
            on_event,
        }
    }

    /// Sets the window that owns the dialogs the OS may show for an operation
    /// (on Windows, the warning that an item cannot go to the trash), as its
    /// raw handle. The window is created after the runtime, so the handle is
    /// given once it exists.
    pub fn set_owner_window(&self, window: Option<isize>) {
        *lock(&self.owner_window) = window;
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
                self.send(Job::ReadDir {
                    side,
                    location,
                    request,
                });
            }
            // Resolving runs on the read thread, since `metadata` on a network
            // path may block.
            Effect::Resolve {
                side,
                path,
                climb,
                request,
            } => {
                // The old watch might keep a vanished directory half alive on
                // Windows; the application watches again once it knows where it is.
                lock(&self.watcher).unwatch(side);
                lock(&self.pending).insert(request);
                self.send(Job::Resolve {
                    side,
                    path,
                    climb,
                    request,
                });
            }
            Effect::CancelRead { request } => {
                lock(&self.pending).remove(&request);
                for lane in [&self.left, &self.right, &self.preview] {
                    lane.cancel(request);
                }
            }
            Effect::LoadPreview { source, request } => {
                lock(&self.pending).insert(request);
                self.send(Job::LoadPreview { source, request });
            }
            Effect::LoadImage { source, request } => {
                lock(&self.pending).insert(request);
                self.send(Job::LoadImage { source, request });
            }
            // A reader abandoned with a preview stuck in it gets no message,
            // but lets go of its archive when its thread ends.
            Effect::ReleasePreview => self.preview.release_preview(),
            Effect::Watch { side, path } => {
                lock(&self.watcher).watch(side, path, &self.on_event);
            }
            Effect::ListDrives { request } => self.list_drives(request),
            Effect::WatchConfig(path) => {
                let watcher = watch_config(&path, &self.paths.home, &self.on_event);
                *lock(&self.config_watcher) = Some(watcher);
            }
            #[cfg(windows)]
            Effect::WatchColorScheme => {
                *lock(&self.color_scheme_watch) = Some(watch_color_scheme(&self.on_event));
            }
            #[cfg(not(windows))]
            Effect::WatchColorScheme => {}
            Effect::LoadShellEnv => self.load_shell_env(),
            Effect::Spawn {
                name,
                program,
                args,
                cwd,
            } => self.spawn(name, program, args, &cwd),
            // A failure here has nowhere to go: the application is quitting.
            Effect::SaveState(state) => {
                let _ = write_state(&self.paths.state, &state);
            }
            Effect::SpawnNewWindow { state, as_admin } => {
                let error = write_handoff(&state)
                    .and_then(|handoff| {
                        spawn_new_window(
                            &handoff,
                            self.paths.config_override.as_deref(),
                            as_admin,
                            self.shell_env(),
                        )
                        .inspect_err(|_| {
                            let _ = fs::remove_file(&handoff);
                        })
                    })
                    .err()
                    .map(|error| IoError::new(error.to_string()));
                (self.on_event)(IoEvent::Spawned {
                    kind: SpawnKind::NewWindow { as_admin },
                    error,
                });
            }
            effect => self.run_file_effect(effect),
        }
    }

    fn send(&self, job: Job) {
        self.lane(&job).send(job);
    }

    fn lane(&self, job: &Job) -> &Lane {
        match job {
            Job::ReadDir { side, .. } | Job::Resolve { side, .. } => match side {
                Side::Left => &self.left,
                Side::Right => &self.right,
            },
            Job::LoadPreview { .. } | Job::LoadImage { .. } => &self.preview,
        }
    }

    // The start is reported before the child is waited for, so that a child
    // that ends at once is not reported as ended first.
    fn spawn(&self, name: SpawnName, program: String, args: Vec<OsString>, cwd: &Path) {
        let child = spawn(&program, &args, Some(cwd), self.shell_env());
        (self.on_event)(IoEvent::Spawned {
            kind: SpawnKind::External {
                name: name.clone(),
                program,
                args,
            },
            error: io_error(child.as_ref()),
        });
        if let Ok(child) = child {
            let on_event = Arc::clone(&self.on_event);
            wait_in_background(child, move |status| {
                on_event(IoEvent::Exited {
                    name,
                    code: status.code(),
                });
            });
        }
    }

    fn shell_env(&self) -> &[(OsString, OsString)] {
        self.shell_env.get().map_or(&[], Vec::as_slice)
    }

    // Started from a terminal, wafu already has the shell's environment. The
    // answer is sent even then, and even after a panic, since the application
    // holds back the programs it starts until it gets one.
    #[cfg(unix)]
    fn load_shell_env(&self) {
        if io::stdin().is_terminal() {
            (self.on_event)(IoEvent::ShellEnvLoaded { error: None });
            return;
        }
        let shell_env = Arc::clone(&self.shell_env);
        let on_event = Arc::clone(&self.on_event);
        thread::Builder::new()
            .name("wafu-io-shell-env".to_owned())
            .spawn(move || {
                let error = match panic::catch_unwind(load_shell_env) {
                    Ok(Ok(vars)) => {
                        let _ = shell_env.set(vars);
                        None
                    }
                    Ok(Err(error)) => Some(IoError::new(error.to_string())),
                    Err(payload) => Some(panic_error(&*payload)),
                };
                on_event(IoEvent::ShellEnvLoaded { error });
            })
            .expect("the shell environment thread should start");
    }

    #[cfg(not(unix))]
    fn load_shell_env(&self) {
        (self.on_event)(IoEvent::ShellEnvLoaded { error: None });
    }

    fn run_file_effect(&self, effect: Effect) {
        match effect {
            Effect::StartOperation(spec) => {
                let (sender, replies) = mpsc::channel();
                *lock(&self.operation) = Some(sender);
                let owner_window = *lock(&self.owner_window);
                let on_event = Arc::clone(&self.on_event);
                thread::Builder::new()
                    .name("wafu-io-operation".to_owned())
                    .spawn(move || {
                        let progress = RefCell::new(None);
                        let report = |event: OperationEvent| {
                            if let OperationEvent::Progress { current, .. } = &event {
                                *progress.borrow_mut() = Some(current.clone());
                            }
                            on_event(IoEvent::Operation(event));
                        };
                        if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(|| {
                            #[cfg(test)]
                            assert_ne!(first_source(&spec), Path::new(PANIC_PATH), "injected");
                            run_operation(&spec, owner_window, &replies, &report);
                        })) {
                            let path = progress.into_inner().unwrap_or_else(|| first_source(&spec));
                            on_event(IoEvent::Operation(OperationEvent::Finished(
                                Outcome::Failed {
                                    path,
                                    error: panic_error(&*payload),
                                },
                            )));
                        }
                    })
                    .expect("the operation thread should start");
            }
            Effect::OperationReply(reply) => {
                if let Some(sender) = lock(&self.operation).as_ref() {
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
                let error = io_error(set_clipboard(&mut lock(&self.clipboard), &text));
                (self.on_event)(IoEvent::ClipboardSet { text, error });
            }
            Effect::OpenWithOs(path) => {
                let error = io_error(open_with_os(&path));
                (self.on_event)(IoEvent::Opened { path, error });
            }
            _ => {}
        }
    }

    // A thread of its own: `metadata` on a root may wait for a drive to spin
    // up, and on a read lane that would hold up the reads queued behind it.
    fn list_drives(&self, request: RequestId) {
        let on_event = Arc::clone(&self.on_event);
        thread::Builder::new()
            .name("wafu-io-drives".to_owned())
            .spawn(move || {
                on_event(IoEvent::Drives {
                    request,
                    letters: panic::catch_unwind(drives).unwrap_or_default(),
                });
            })
            .expect("the drives thread should start");
    }
}

pub(crate) type OnEvent = Arc<dyn Fn(IoEvent) + Send + Sync>;

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

impl Job {
    fn request(&self) -> RequestId {
        match self {
            Job::ReadDir { request, .. }
            | Job::Resolve { request, .. }
            | Job::LoadPreview { request, .. }
            | Job::LoadImage { request, .. } => *request,
        }
    }

    fn failed(&self, error: IoError) -> IoEvent {
        let request = self.request();
        match self {
            Job::ReadDir { side, .. } => IoEvent::DirLoaded {
                side: *side,
                request,
                result: Err(error),
            },
            Job::Resolve { side, .. } => IoEvent::Resolved {
                side: *side,
                request,
                result: None,
            },
            Job::LoadPreview { .. } => IoEvent::PreviewLoaded {
                request,
                result: Err(PreviewError::Failed(error)),
            },
            Job::LoadImage { .. } => IoEvent::ImageLoaded {
                request,
                result: Err(PreviewError::Failed(error)),
            },
        }
    }
}

// A queue of jobs run in order on one thread. A cancelled job may be stuck
// in a blocked read, so its thread is abandoned (its result, if it ever
// comes, is dropped) and a new one takes over the queue. The preview lane
// stops abandoning at a limit, so that racing through images decodes only a
// few at once.
struct Lane {
    name: &'static str,
    limit: Option<usize>,
    abandoned: Arc<AtomicUsize>,
    pending: Arc<Mutex<HashSet<RequestId>>>,
    on_event: OnEvent,
    reader: Mutex<Reader>,
}

impl Lane {
    fn new(
        name: &'static str,
        limit: Option<usize>,
        pending: &Arc<Mutex<HashSet<RequestId>>>,
        on_event: &OnEvent,
    ) -> Self {
        let abandoned = Arc::new(AtomicUsize::new(0));
        let reader = Reader::spawn(name, &abandoned, pending, on_event);
        Self {
            name,
            limit,
            abandoned,
            pending: Arc::clone(pending),
            on_event: Arc::clone(on_event),
            reader: Mutex::new(reader),
        }
    }

    fn send(&self, job: Job) {
        let _ = lock(&self.reader).jobs.send(Queued::Job(job));
    }

    fn release_preview(&self) {
        let _ = lock(&self.reader).jobs.send(Queued::ReleasePreview);
    }

    fn cancel(&self, request: RequestId) {
        let mut reader = lock(&self.reader);
        if *lock(&reader.running) != Some(request)
            || self
                .limit
                .is_some_and(|limit| self.abandoned.load(Ordering::SeqCst) >= limit)
        {
            return;
        }
        reader.abandoned.store(true, Ordering::SeqCst);
        self.abandoned.fetch_add(1, Ordering::SeqCst);
        *reader = Reader::spawn(self.name, &self.abandoned, &self.pending, &self.on_event);
    }
}

enum Queued {
    Job(Job),
    ReleasePreview,
}

struct Reader {
    jobs: Sender<Queued>,
    running: Arc<Mutex<Option<RequestId>>>,
    abandoned: Arc<AtomicBool>,
}

impl Reader {
    fn spawn(
        name: &str,
        lane_abandoned: &Arc<AtomicUsize>,
        pending: &Arc<Mutex<HashSet<RequestId>>>,
        on_event: &OnEvent,
    ) -> Self {
        let (jobs, receiver) = mpsc::channel::<Queued>();
        let running = Arc::new(Mutex::new(None));
        let abandoned = Arc::new(AtomicBool::new(false));
        let reader = Self {
            jobs,
            running: Arc::clone(&running),
            abandoned: Arc::clone(&abandoned),
        };
        let lane_abandoned = Arc::clone(lane_abandoned);
        let pending = Arc::clone(pending);
        let on_event = Arc::clone(on_event);
        thread::Builder::new()
            .name(format!("wafu-io-reader-{name}"))
            .spawn(move || {
                let mut cache: Option<CachedArchive> = None;
                let mut previewed = PreviewedArchive::default();
                for queued in receiver {
                    let job = match queued {
                        Queued::Job(job) => job,
                        Queued::ReleasePreview => {
                            previewed = PreviewedArchive::default();
                            continue;
                        }
                    };
                    let request = job.request();
                    *lock(&running) = Some(request);
                    let live = lock(&pending).contains(&request);
                    let event = live.then(|| {
                        panic::catch_unwind(AssertUnwindSafe(|| {
                            read(&job, &mut cache, &mut previewed, &pending)
                        }))
                        .unwrap_or_else(|payload| job.failed(panic_error(&*payload)))
                    });
                    *lock(&running) = None;
                    if let Some(event) = event
                        && lock(&pending).remove(&request)
                    {
                        on_event(event);
                    }
                }
                if abandoned.load(Ordering::SeqCst) {
                    lane_abandoned.fetch_sub(1, Ordering::SeqCst);
                }
            })
            .expect("the reader thread should start");
        reader
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

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn read(
    job: &Job,
    cache: &mut Option<CachedArchive>,
    previewed: &mut PreviewedArchive,
    pending: &Mutex<HashSet<RequestId>>,
) -> IoEvent {
    let request = job.request();
    match job {
        Job::ReadDir { side, location, .. } => {
            #[cfg(test)]
            assert_ne!(location.base, Path::new(PANIC_PATH), "injected");
            let cancelled = || !lock(pending).contains(&request);
            let result = match &location.archive {
                None => read_dir_with(&location.base, cancelled),
                Some(archive) => list_archive(cache, &location.base, archive, cancelled),
            }
            .map_err(|error| IoError::new(error.to_string()));
            IoEvent::DirLoaded {
                side: *side,
                request,
                result,
            }
        }
        Job::Resolve {
            side, path, climb, ..
        } => IoEvent::Resolved {
            side: *side,
            request,
            result: resolve(path, *climb),
        },
        Job::LoadPreview { source, .. } => IoEvent::PreviewLoaded {
            request,
            result: load_preview(source, previewed),
        },
        Job::LoadImage { source, .. } => IoEvent::ImageLoaded {
            request,
            result: load_image(source, previewed),
        },
    }
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
    let cached = match cache {
        Some(cached) if fresh => cached,
        _ => {
            let reader = open_archive(path, archive.format).map_err(io::Error::other)?;
            cache.insert(CachedArchive {
                path: path.to_path_buf(),
                len,
                modified,
                entries: reader.entries().to_vec(),
            })
        }
    };
    let entries = &cached.entries;
    let own = dir_entry(entries, &archive.inner).or_else(|| own_entry(path));
    let entries = list_dir(entries, &archive.inner, cancelled)?;
    Ok(Directory { own, entries })
}

// A path that no file system accepts, so a test can make a worker panic on it.
#[cfg(test)]
const PANIC_PATH: &str = "\0panic";

fn first_source(spec: &OperationSpec) -> PathBuf {
    match &spec.sources {
        Sources::Paths(paths) => paths.first().cloned().unwrap_or_default(),
        Sources::Entries(entries) => entries.archive.clone(),
    }
}

fn panic_error(payload: &(dyn Any + Send)) -> IoError {
    let message = payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("unknown cause");
    IoError::new(format!("panicked: {message}"))
}

fn io_error<T, E: Display>(result: Result<T, E>) -> Option<IoError> {
    result.err().map(|error| IoError::new(error.to_string()))
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs::{self, File};
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;
    use std::sync::mpsc::Receiver;
    use std::time::Duration;

    use tempfile::TempDir;
    use wafu_core::{Command, ConfigContent, ConfigFile};

    use super::*;
    use crate::watch::tests::symlink;

    const TIMEOUT: Duration = Duration::from_secs(5);
    const QUIET: Duration = Duration::from_millis(600);

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

        with_worker_held(&runtime, |pending, send| {
            pending.insert(RequestId(1));
            send(read_job(dir.path(), 1));
            pending.remove(&RequestId(1));
            pending.insert(RequestId(2));
            send(read_job(dir.path(), 2));
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

    #[cfg(unix)]
    #[test]
    fn cancelling_a_stuck_read_lets_the_next_read_through() {
        let dir = TempDir::new().unwrap();
        let fifo = fifo(dir.path(), "stuck.zip");
        let (runtime, events) = runtime();

        runtime.run(read_archive(&fifo, Side::Left, 1));
        wait_until(|| running(&runtime.left) == Some(RequestId(1)));
        runtime.run(Effect::CancelRead {
            request: RequestId(1),
        });
        runtime.run(read(dir.path(), 2));

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::DirLoaded { request, .. } = event else {
            panic!("expected DirLoaded, got {event:?}");
        };
        assert_eq!(request, RequestId(2));
        release(&fifo);
        wait_until(|| runtime.left.abandoned.load(Ordering::SeqCst) == 0);
        assert!(events.recv_timeout(Duration::from_millis(100)).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_stuck_read_in_one_pane_does_not_hold_up_the_other() {
        let dir = TempDir::new().unwrap();
        let fifo = fifo(dir.path(), "stuck.zip");
        let (runtime, events) = runtime();

        runtime.run(read_archive(&fifo, Side::Left, 1));
        runtime.run(Effect::ReadDir {
            side: Side::Right,
            location: Location::new(dir.path().to_path_buf()),
            request: RequestId(2),
        });

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::DirLoaded { side, request, .. } = event else {
            panic!("expected DirLoaded, got {event:?}");
        };
        assert_eq!((side, request), (Side::Right, RequestId(2)));
        release(&fifo);
        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::DirLoaded { side, request, .. } = event else {
            panic!("expected DirLoaded, got {event:?}");
        };
        assert_eq!((side, request), (Side::Left, RequestId(1)));
    }

    #[cfg(unix)]
    #[test]
    fn the_preview_lane_waits_once_two_readers_are_abandoned() {
        let dir = TempDir::new().unwrap();
        let text = dir.path().join("a.txt");
        fs::write(&text, b"hello\n").unwrap();
        let fifos: Vec<PathBuf> = ["1", "2", "3"]
            .into_iter()
            .map(|name| fifo(dir.path(), name))
            .collect();
        let (runtime, events) = runtime();

        for (request, fifo) in (1..).map(RequestId).zip(&fifos) {
            runtime.run(load_preview_of(fifo, request));
            wait_until(|| running(&runtime.preview) == Some(request));
            runtime.run(Effect::CancelRead { request });
        }
        runtime.run(load_preview_of(&text, RequestId(4)));

        assert!(events.recv_timeout(Duration::from_millis(200)).is_err());
        release(&fifos[2]);
        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::PreviewLoaded { request, .. } = event else {
            panic!("expected PreviewLoaded, got {event:?}");
        };
        assert_eq!(request, RequestId(4));
        release(&fifos[0]);
        release(&fifos[1]);
        wait_until(|| runtime.preview.abandoned.load(Ordering::SeqCst) == 0);
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
    fn releasing_the_preview_makes_the_next_preview_read_the_archive_again() {
        let dir = TempDir::new().unwrap();
        let archive = dir.path().join("a.tar");
        let mut writer = crate::archive::create_archive(
            File::create(&archive).unwrap(),
            wafu_core::ArchiveFormat::Tar,
            None,
        )
        .unwrap();
        writer
            .add_file("a.txt", 1, None, None, &mut &b"a"[..])
            .unwrap();
        writer
            .add_file("b.txt", 1, None, None, &mut &b"b"[..])
            .unwrap();
        writer.finish().unwrap();
        let (runtime, events) = runtime();
        let load = |name: &str, request| Effect::LoadPreview {
            source: PreviewSource {
                path: archive.clone(),
                entry: Some((
                    ArchivePath {
                        format: Some(wafu_core::ArchiveFormat::Tar),
                        inner: Vec::new(),
                    },
                    name.to_owned(),
                )),
                password: None,
            },
            request: RequestId(request),
        };
        runtime.run(load("a.txt", 1));
        let IoEvent::PreviewLoaded { result, .. } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected PreviewLoaded");
        };
        assert!(result.is_ok());

        // The first header made unreadable without the archive looking
        // changed, so that only a new index notices.
        let modified = fs::metadata(&archive).unwrap().modified().unwrap();
        let file = File::options().write(true).open(&archive).unwrap();
        std::io::Write::write_all(&mut &file, &[0; 512]).unwrap();
        file.set_modified(modified).unwrap();
        drop(file);
        runtime.run(Effect::ReleasePreview);
        runtime.run(load("b.txt", 2));

        let IoEvent::PreviewLoaded { result, .. } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected PreviewLoaded");
        };
        assert!(result.is_err());
    }

    #[test]
    fn list_drives_reports_the_letters() {
        let (runtime, events) = runtime();

        runtime.run(Effect::ListDrives {
            request: RequestId(1),
        });

        let IoEvent::Drives { request, letters } = events.recv_timeout(TIMEOUT).unwrap() else {
            panic!("expected Drives");
        };
        assert_eq!(request, RequestId(1));
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
    fn changes_in_a_directory_made_again_at_the_watched_path_are_reported() {
        let parent = TempDir::new().unwrap();
        let dir = parent.path().join("watched");
        fs::create_dir(&dir).unwrap();
        let (runtime, events) = runtime();
        runtime.run(watch(&dir));
        settle(&events);

        fs::remove_dir(&dir).unwrap();
        fs::create_dir(&dir).unwrap();
        let changed = IoEvent::LocationChanged {
            side: Side::Left,
            path: dir.clone(),
        };
        assert_eq!(events.recv_timeout(TIMEOUT).unwrap(), changed);
        settle(&events);

        File::create(dir.join("a")).unwrap();

        assert_eq!(events.recv_timeout(TIMEOUT).unwrap(), changed);
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
    fn writing_the_target_of_a_watched_link_to_a_file_is_reported() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("other")).unwrap();
        let real = dir.path().join("other").join("b.zip");
        fs::write(&real, b"v1").unwrap();
        let link = dir.path().join("a.zip");
        if !symlink(&real, &link) {
            return;
        }
        let (runtime, events) = runtime();
        runtime.run(watch(&link));
        settle(&events);

        fs::write(&real, b"v2").unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            IoEvent::LocationChanged {
                side: Side::Left,
                path: link,
            }
        );
    }

    #[test]
    fn removing_the_target_of_a_watched_link_to_a_directory_is_reported() {
        assert_removing_the_target_is_reported(symlink);
    }

    #[test]
    fn changes_in_the_new_target_of_a_replaced_link_are_reported() {
        let dir = TempDir::new().unwrap();
        let old = dir.path().join("targets").join("old");
        let new = dir.path().join("targets").join("new");
        fs::create_dir_all(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        let link = dir.path().join("link");
        if !symlink(&old, &link) {
            return;
        }
        let (runtime, events) = runtime();
        runtime.run(watch(&link));
        settle(&events);

        let temporary = dir.path().join("link.tmp");
        assert!(symlink(&new, &temporary));
        fs::rename(&temporary, &link).unwrap();
        let changed = IoEvent::LocationChanged {
            side: Side::Left,
            path: link.clone(),
        };
        assert_eq!(events.recv_timeout(TIMEOUT).unwrap(), changed);
        settle(&events);

        File::create(new.join("a")).unwrap();

        assert_eq!(events.recv_timeout(TIMEOUT).unwrap(), changed);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failing_to_watch_the_target_of_a_link_is_reported() {
        let dir = TempDir::new().unwrap();
        let locked = dir.path().join("locked");
        let real = locked.join("real");
        fs::create_dir_all(&real).unwrap();
        let link = dir.path().join("link");
        assert!(symlink(&real, &link));
        let Some(_unreadable) = Unreadable::new(&locked) else {
            return;
        };
        let (runtime, events) = runtime();

        runtime.run(watch(&link));

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::WatchFailed { side, path, .. } = event else {
            panic!("expected WatchFailed, got {event:?}");
        };
        assert_eq!(side, Side::Left);
        assert_eq!(path, link);
        settle(&events);

        File::create(real.join("a")).unwrap();

        assert_eq!(
            events.recv_timeout(TIMEOUT).unwrap(),
            IoEvent::LocationChanged {
                side: Side::Left,
                path: link,
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failing_to_watch_the_new_target_of_a_replaced_link_is_reported_once() {
        let dir = TempDir::new().unwrap();
        let old = dir.path().join("old");
        let locked = dir.path().join("locked");
        let new = locked.join("new");
        fs::create_dir(&old).unwrap();
        fs::create_dir_all(&new).unwrap();
        let link = dir.path().join("link");
        assert!(symlink(&old, &link));
        let Some(_unreadable) = Unreadable::new(&locked) else {
            return;
        };
        let (runtime, events) = runtime();
        runtime.run(watch(&link));
        settle(&events);

        let temporary = dir.path().join("link.tmp");
        assert!(symlink(&new, &temporary));
        fs::rename(&temporary, &link).unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::WatchFailed { side, path, .. } = event else {
            panic!("expected WatchFailed, got {event:?}");
        };
        assert_eq!(side, Side::Left);
        assert_eq!(path, link);
        let changed = IoEvent::LocationChanged {
            side: Side::Left,
            path: link.clone(),
        };
        assert_eq!(events.recv_timeout(TIMEOUT).unwrap(), changed);
        settle(&events);

        File::create(new.join("a")).unwrap();

        assert_eq!(events.recv_timeout(TIMEOUT).unwrap(), changed);
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
            )
            .unwrap();
            writer
                .add_file("d/f.txt", 1, None, None, &mut &b"f"[..])
                .unwrap();
            writer.finish().unwrap();
        }
        let (runtime, events) = runtime();
        let root = Location::archive_root(archive.clone(), Some(wafu_core::ArchiveFormat::Zip));

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

    #[cfg(windows)]
    #[test]
    fn the_color_scheme_is_reported_as_soon_as_it_is_watched() {
        let (runtime, events) = runtime();

        runtime.run(Effect::WatchColorScheme);

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::ColorScheme { .. } = event else {
            panic!("expected ColorScheme, got {event:?}");
        };
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
        assert_eq!(event, config_loaded(&[(&path, "a = 2\n")]));
        assert!(events.recv_timeout(QUIET).is_err());

        fs::write(dir.path().join("other"), "x").unwrap();
        assert!(events.recv_timeout(QUIET).is_err());
    }

    #[test]
    fn config_is_read_again_only_when_its_modification_time_changes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "a = 1\n").unwrap();
        let (runtime, events) = runtime();
        runtime.run(Effect::WatchConfig(path.clone()));
        settle(&events);

        let original = fs::metadata(&path).unwrap().permissions();
        let mut readonly = original.clone();
        readonly.set_readonly(true);
        fs::set_permissions(&path, readonly).unwrap();
        assert!(events.recv_timeout(QUIET).is_err());
        fs::set_permissions(&path, original).unwrap();
        assert!(events.recv_timeout(QUIET).is_err());

        let temp = dir.path().join("config.toml.tmp");
        fs::write(&temp, "a = 1\n").unwrap();
        fs::rename(&temp, &path).unwrap();
        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(event, config_loaded(&[(&path, "a = 1\n")]));
    }

    #[test]
    fn replacing_the_target_of_a_linked_config_reports_the_new_text() {
        let dir = TempDir::new().unwrap();
        let dotfiles = dir.path().join("dotfiles");
        fs::create_dir(&dotfiles).unwrap();
        let real = dotfiles.join("config.toml");
        fs::write(&real, "a = 1\n").unwrap();
        let link = dir.path().join("config.toml");
        if !symlink(&real, &link) {
            return;
        }
        let (runtime, events) = runtime();
        runtime.run(Effect::WatchConfig(link.clone()));
        settle(&events);

        let temp = dotfiles.join("config.toml.tmp");
        fs::write(&temp, "a = 2\n").unwrap();
        fs::rename(&temp, &real).unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(event, config_loaded(&[(&link, "a = 2\n")]));
    }

    #[test]
    fn editing_a_base_of_the_config_reports_the_whole_chain() {
        let dir = TempDir::new().unwrap();
        let shared = dir.path().join("shared");
        fs::create_dir(&shared).unwrap();
        let base = shared.join("common.toml");
        fs::write(&base, "a = 1\n").unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "extends = \"shared/common.toml\"\n").unwrap();
        let (runtime, events) = runtime();
        runtime.run(Effect::WatchConfig(path.clone()));
        settle(&events);

        let temp = shared.join("common.toml.tmp");
        fs::write(&temp, "a = 2\n").unwrap();
        fs::rename(&temp, &base).unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            config_loaded(&[
                (&base, "a = 2\n"),
                (&path, "extends = \"shared/common.toml\"\n")
            ])
        );
    }

    #[test]
    fn a_base_named_on_a_reload_is_watched_from_then_on() {
        let dir = TempDir::new().unwrap();
        let shared = dir.path().join("shared");
        fs::create_dir(&shared).unwrap();
        let base = shared.join("common.toml");
        let path = dir.path().join("config.toml");
        fs::write(&path, "a = 1\n").unwrap();
        let (runtime, events) = runtime();
        runtime.run(Effect::WatchConfig(path.clone()));
        settle(&events);

        fs::write(&path, "extends = \"shared/common.toml\"\n").unwrap();
        events.recv_timeout(TIMEOUT).unwrap();
        settle(&events);
        fs::write(&base, "a = 2\n").unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            config_loaded(&[
                (&base, "a = 2\n"),
                (&path, "extends = \"shared/common.toml\"\n")
            ])
        );
    }

    #[test]
    fn a_base_whose_directory_appears_later_is_watched_after_the_next_reload() {
        let dir = TempDir::new().unwrap();
        let shared = dir.path().join("shared");
        let base = shared.join("common.toml");
        let path = dir.path().join("config.toml");
        fs::write(&path, "extends = \"shared/common.toml\"\n").unwrap();
        let (runtime, events) = runtime();
        runtime.run(Effect::WatchConfig(path.clone()));
        settle(&events);

        fs::create_dir(&shared).unwrap();
        fs::write(&base, "a = 1\n").unwrap();
        fs::write(&path, "extends = \"shared/common.toml\"\n").unwrap();
        events.recv_timeout(TIMEOUT).unwrap();
        settle(&events);
        fs::write(&base, "a = 2\n").unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            config_loaded(&[
                (&base, "a = 2\n"),
                (&path, "extends = \"shared/common.toml\"\n")
            ])
        );
    }

    #[test]
    fn a_base_in_a_missing_directory_is_not_reported_and_the_config_is_still_watched() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, "extends = \"missing/common.toml\"\n").unwrap();
        let (runtime, events) = runtime();
        runtime.run(Effect::WatchConfig(path.clone()));
        let failures: Vec<IoEvent> = events
            .try_iter()
            .filter(|event| matches!(event, IoEvent::ConfigWatchFailed { .. }))
            .collect();
        assert_eq!(failures, []);
        settle(&events);

        fs::write(&path, "a = 1\n").unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(event, config_loaded(&[(&path, "a = 1\n")]));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failing_to_watch_the_target_of_a_linked_config_is_reported() {
        let dir = TempDir::new().unwrap();
        let dotfiles = dir.path().join("dotfiles");
        fs::create_dir(&dotfiles).unwrap();
        let real = dotfiles.join("config.toml");
        fs::write(&real, "a = 1\n").unwrap();
        let link = dir.path().join("config.toml");
        assert!(symlink(&real, &link));
        let Some(_unreadable) = Unreadable::new(&dotfiles) else {
            return;
        };
        let (runtime, events) = runtime();

        runtime.run(Effect::WatchConfig(link));

        let event = events.recv_timeout(TIMEOUT).unwrap();
        let IoEvent::ConfigWatchFailed { .. } = event else {
            panic!("expected ConfigWatchFailed, got {event:?}");
        };
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
                home: dir.path().to_path_buf(),
            },
            move |event| {
                let _ = lock(&sender).send(event);
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
            name: SpawnName::Command(Command::External("x".to_owned())),
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
                name: SpawnName::Command(Command::External("x".to_owned())),
                program: "wafu-no-such-program".to_owned(),
                args: Vec::new(),
            }
        );
        assert!(error.is_some());
    }

    #[test]
    fn a_spawn_reports_its_start_and_then_its_exit() {
        let dir = TempDir::new().unwrap();
        let (runtime, events) = runtime();
        let (program, args) = if cfg!(windows) {
            ("cmd", vec!["/c", "exit", "1"])
        } else {
            ("false", vec![])
        };
        let args: Vec<OsString> = args.into_iter().map(OsString::from).collect();

        runtime.run(Effect::Spawn {
            name: SpawnName::Command(Command::External("x".to_owned())),
            program: program.to_owned(),
            args: args.clone(),
            cwd: dir.path().to_path_buf(),
        });

        assert_eq!(
            events.recv_timeout(TIMEOUT).unwrap(),
            IoEvent::Spawned {
                kind: SpawnKind::External {
                    name: SpawnName::Command(Command::External("x".to_owned())),
                    program: program.to_owned(),
                    args,
                },
                error: None,
            }
        );
        assert_eq!(
            events.recv_timeout(TIMEOUT).unwrap(),
            IoEvent::Exited {
                name: SpawnName::Command(Command::External("x".to_owned())),
                code: Some(1),
            }
        );
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
                    assert!(matches!(outcome, wafu_core::Outcome::Completed { .. }));
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

    #[test]
    fn a_panic_on_the_operation_thread_is_reported_as_a_failure_and_frees_the_slot() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("a"), b"a").unwrap();
        let (runtime, events) = runtime();

        runtime.run(Effect::StartOperation(wafu_core::OperationSpec {
            kind: wafu_core::OperationKind::Delete,
            sources: wafu_core::Sources::Paths(vec![PathBuf::from(PANIC_PATH)]),
            dest: None,
            output: None,
        }));

        let IoEvent::Operation(wafu_core::OperationEvent::Finished(outcome)) =
            events.recv_timeout(TIMEOUT).unwrap()
        else {
            panic!("expected Finished");
        };
        let wafu_core::Outcome::Failed { path, error } = outcome else {
            panic!("expected Failed, got {outcome:?}");
        };
        assert_eq!(path, Path::new(PANIC_PATH));
        assert!(error.to_string().starts_with("panicked: "), "{error}");

        runtime.run(Effect::StartOperation(wafu_core::OperationSpec {
            kind: wafu_core::OperationKind::Delete,
            sources: wafu_core::Sources::Paths(vec![dir.path().join("a")]),
            dest: None,
            output: None,
        }));
        loop {
            if let IoEvent::Operation(wafu_core::OperationEvent::Finished(outcome)) =
                events.recv_timeout(TIMEOUT).unwrap()
            {
                assert!(matches!(outcome, wafu_core::Outcome::Completed { .. }));
                break;
            }
        }
    }

    #[test]
    fn a_panic_on_a_worker_is_reported_as_an_error_and_the_worker_goes_on() {
        let dir = TempDir::new().unwrap();
        let (runtime, events) = runtime();

        runtime.run(read(Path::new(PANIC_PATH), 1));
        runtime.run(read(dir.path(), 2));

        let IoEvent::DirLoaded {
            request, result, ..
        } = events.recv_timeout(TIMEOUT).unwrap()
        else {
            panic!("expected DirLoaded");
        };
        assert_eq!(request, RequestId(1));
        assert!(
            result
                .as_ref()
                .is_err_and(|error| error.to_string().starts_with("panicked: ")),
            "{result:?}"
        );
        let IoEvent::DirLoaded {
            request, result, ..
        } = events.recv_timeout(TIMEOUT).unwrap()
        else {
            panic!("expected DirLoaded");
        };
        assert_eq!(request, RequestId(2));
        assert!(result.is_ok());
    }

    #[test]
    fn a_panic_becomes_an_error_with_its_message() {
        let literal = panic::catch_unwind(|| panic!("boom")).unwrap_err();
        let formatted = panic::catch_unwind(|| panic!("boom {}", 1)).unwrap_err();

        assert_eq!(panic_error(&*literal), IoError::new("panicked: boom"));
        assert_eq!(panic_error(&*formatted), IoError::new("panicked: boom 1"));
    }

    // A watch takes a moment to become active, and FSEvents on macOS may then
    // report changes made just before it started (such as creating the
    // directory itself), so wait until nothing has arrived for a while.
    fn settle(events: &Receiver<IoEvent>) {
        std::thread::sleep(Duration::from_millis(200));
        while events.recv_timeout(QUIET).is_ok() {}
    }

    fn config_loaded(files: &[(&Path, &str)]) -> IoEvent {
        IoEvent::ConfigLoaded {
            files: files
                .iter()
                .map(|(path, text)| ConfigFile {
                    path: path.to_path_buf(),
                    content: ConfigContent::Text((*text).to_owned()),
                })
                .collect(),
        }
    }

    fn runtime() -> (Runtime, Receiver<IoEvent>) {
        let (sender, receiver) = mpsc::channel();
        let sender = Mutex::new(sender);
        let paths = Paths {
            state: std::env::temp_dir().join("wafu-test-state.toml"),
            config_override: None,
            home: std::env::temp_dir(),
        };
        let runtime = Runtime::new(paths, move |event| {
            let _ = lock(&sender).send(event);
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

    // Queues jobs and cancellations as `run` would, but with the workers held
    // off the pending set until all of them are in, so that a cancellation
    // cannot lose the race against a read that finishes at once.
    fn with_worker_held(
        runtime: &Runtime,
        queue: impl FnOnce(&mut HashSet<RequestId>, &dyn Fn(Job)),
    ) {
        let mut pending = lock(&runtime.pending);
        queue(&mut pending, &|job| runtime.send(job));
    }

    #[cfg(unix)]
    fn read_archive(path: &Path, side: Side, request: u64) -> Effect {
        Effect::ReadDir {
            side,
            location: Location::archive_root(
                path.to_path_buf(),
                Some(wafu_core::ArchiveFormat::Zip),
            ),
            request: RequestId(request),
        }
    }

    #[cfg(unix)]
    fn load_preview_of(path: &Path, request: RequestId) -> Effect {
        Effect::LoadPreview {
            source: PreviewSource {
                path: path.to_path_buf(),
                entry: None,
                password: None,
            },
            request,
        }
    }

    #[cfg(unix)]
    fn fifo(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        let status = std::process::Command::new("mkfifo")
            .arg(&path)
            .status()
            .unwrap();
        assert!(status.success());
        path
    }

    #[cfg(unix)]
    fn release(fifo: &Path) {
        fs::OpenOptions::new().write(true).open(fifo).unwrap();
    }

    #[cfg(unix)]
    fn running(lane: &Lane) -> Option<RequestId> {
        *lock(&lock(&lane.reader).running)
    }

    #[cfg(unix)]
    fn wait_until(condition: impl Fn() -> bool) {
        let deadline = std::time::Instant::now() + TIMEOUT;
        while !condition() {
            assert!(std::time::Instant::now() < deadline, "timed out");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn assert_removing_the_target_is_reported(link_to: impl FnOnce(&Path, &Path) -> bool) {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("other").join("real");
        fs::create_dir_all(&real).unwrap();
        let link = dir.path().join("link");
        if !link_to(&real, &link) {
            return;
        }
        let (runtime, events) = runtime();
        runtime.run(watch(&link));
        settle(&events);

        fs::remove_dir(&real).unwrap();

        let event = events.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(
            event,
            IoEvent::LocationChanged {
                side: Side::Left,
                path: link,
            }
        );
    }

    fn watch(path: &Path) -> Effect {
        Effect::Watch {
            side: Side::Left,
            path: path.to_path_buf(),
        }
    }

    #[cfg(target_os = "linux")]
    struct Unreadable(PathBuf);

    #[cfg(target_os = "linux")]
    impl Unreadable {
        fn new(dir: &Path) -> Option<Self> {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o100)).unwrap();
            let unreadable = Self(dir.to_path_buf());
            fs::read_dir(dir).is_err().then_some(unreadable)
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for Unreadable {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o755));
        }
    }
}
