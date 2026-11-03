use std::ffi::OsStr;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

#[cfg(target_os = "macos")]
use icu_normalizer::ComposingNormalizer;
use notify::event::{AccessKind, AccessMode, ModifyKind};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher as _};
use wafu_core::{IoError, IoEvent, Side};

use crate::runtime::{OnEvent, lock};

const DEBOUNCE: Duration = Duration::from_millis(200);
const MAX_WAIT: Duration = Duration::from_secs(1);

// Each pane has its own group of watches and debounce thread so that the
// callback knows which pane and path it reports for.
pub(crate) struct Watcher {
    sides: [Option<Group>; 2],
}

impl Watcher {
    pub(crate) fn new() -> Self {
        Self {
            sides: [None, None],
        }
    }

    pub(crate) fn watch(&mut self, side: Side, path: PathBuf, on_event: &OnEvent) {
        self.unwatch(side);
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
        let (changes, receiver) = mpsc::channel::<Change>();
        let (followed, mut failure) = follow(&path, resolve_link(&path), is_dir, &changes);
        // The directory's own watch goes quiet when the directory is removed
        // or renamed (Windows reports nothing at all), so the parent is
        // watched for that one entry as well. For a directory, failing to is
        // not worth a report; for a file it is the only watch there is besides
        // the target of a link.
        let entry = match path
            .parent()
            .map(|parent| watch_entry(parent, &path, is_dir, changes.clone()))
        {
            Some(Ok(watcher)) => Some(watcher),
            Some(Err(error)) if !is_dir => {
                failure.get_or_insert(error);
                None
            }
            _ => None,
        };
        let report = Arc::clone(on_event);
        let on_failure = move |path: &Path, error: notify::Error| {
            report(IoEvent::WatchFailed {
                side,
                path: path.to_path_buf(),
                error: IoError::new(error.to_string()),
            });
        };
        if let Some(error) = failure {
            on_failure(&path, error);
        }
        let watches = Watches {
            _entry: entry,
            followed,
            changes,
        };
        let report = Arc::clone(on_event);
        let on_burst = move |path: &Path| {
            report(IoEvent::LocationChanged {
                side,
                path: path.to_path_buf(),
            });
        };
        self.sides[side.index()] = Some(Group::start(
            path, true, watches, receiver, on_burst, on_failure,
        ));
    }

    pub(crate) fn unwatch(&mut self, side: Side) {
        self.sides[side.index()] = None;
    }
}

// The watches for one path and the debounce thread that reports for them.
// The thread holds the watches only weakly and every sender lives inside
// them, so dropping the group ends the thread.
pub(crate) struct Group {
    _watches: Arc<Mutex<Watches>>,
}

impl Group {
    fn start(
        path: PathBuf,
        with_contents: bool,
        watches: Watches,
        changes: Receiver<Change>,
        mut on_burst: impl FnMut(&Path) + Send + 'static,
        on_failure: impl Fn(&Path, notify::Error) + Send + 'static,
    ) -> Self {
        let watches = Arc::new(Mutex::new(watches));
        let weak = Arc::downgrade(&watches);
        spawn_debouncer(changes, move |renamed| {
            if !refollow(&weak, &path, with_contents, renamed, |error| {
                on_failure(&path, error);
            }) {
                return false;
            }
            on_burst(&path);
            true
        });
        Self { _watches: watches }
    }
}

struct Watches {
    _entry: Option<RecommendedWatcher>,
    followed: Followed,
    changes: Sender<Change>,
}

// The watches that depend on where the path leads, made again when it leads
// elsewhere or the directory there is renamed, removed, or made.
struct Followed {
    resolution: Resolution,
    _contents: Option<RecommendedWatcher>,
    _target: Option<RecommendedWatcher>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolution {
    NotLink,
    Target(PathBuf),
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Contents,
    Name,
}

// Editors often replace the file rather than write into it, so its directory
// is watched and the file is read again after each burst of changes.
pub(crate) fn watch_config(path: &Path, on_event: &OnEvent) -> notify::Result<Group> {
    let parent = path
        .parent()
        .ok_or_else(|| notify::Error::generic("the configuration file has no parent directory"))?;
    let (changes, receiver) = mpsc::channel::<Change>();
    let entry = watch_entry(parent, path, false, changes.clone())?;
    let report = Arc::clone(on_event);
    let on_failure = move |_: &Path, error: notify::Error| {
        report(IoEvent::ConfigWatchFailed {
            error: IoError::new(error.to_string()),
        });
    };
    let (followed, failure) = follow(path, resolve_link(path), false, &changes);
    if let Some(error) = failure {
        on_failure(path, error);
    }
    let watches = Watches {
        _entry: Some(entry),
        followed,
        changes,
    };
    let report = Arc::clone(on_event);
    let mut last_modified = modified(path);
    let on_burst = move |path: &Path| {
        let now_modified = modified(path);
        if now_modified.is_some() && now_modified == last_modified {
            return;
        }
        last_modified = now_modified;
        let result = fs::read_to_string(path).map_err(|error| IoError::new(error.to_string()));
        report(IoEvent::ConfigLoaded { result });
    };
    Ok(Group::start(
        path.to_path_buf(),
        false,
        watches,
        receiver,
        on_burst,
        on_failure,
    ))
}

fn modified(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|meta| meta.modified()).ok()
}

// Whether the last component of the path is a link, and if so, what it
// finally leads to. Comparing the path with its canonical form would not do:
// they differ without any link (`\\?\` on Windows, `/private/var` on macOS).
fn resolve_link(path: &Path) -> Resolution {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::canonicalize(path).map_or(Resolution::Failed, Resolution::Target)
        }
        _ => Resolution::NotLink,
    }
}

// A watch through a link does not see its target removed on Windows, nor a
// file it leads to being written on any system, so the final target is also
// watched from its own directory.
fn follow(
    path: &Path,
    resolution: Resolution,
    is_dir: bool,
    changes: &Sender<Change>,
) -> (Followed, Option<notify::Error>) {
    let mut failure = None;
    let mut keep = |result: notify::Result<RecommendedWatcher>| match result {
        Ok(watcher) => Some(watcher),
        Err(error) => {
            failure.get_or_insert(error);
            None
        }
    };
    let contents = if is_dir {
        keep(watch_contents(path, changes.clone()))
    } else {
        None
    };
    let target = match &resolution {
        Resolution::Target(real) => real
            .parent()
            .and_then(|dir| keep(watch_entry(dir, real, is_dir, changes.clone()))),
        Resolution::NotLink | Resolution::Failed => None,
    };
    let followed = Followed {
        resolution,
        _contents: contents,
        _target: target,
    };
    (followed, failure)
}

// A watch of the contents stays on the old directory when the link is
// replaced or the directory is made again under the same name, so the watches
// are made again once the path leads elsewhere or a watched name changes.
// Resolving and watching may block, so neither happens under the lock.
fn refollow(
    group: &Weak<Mutex<Watches>>,
    path: &Path,
    with_contents: bool,
    renamed: bool,
    on_failure: impl FnOnce(notify::Error),
) -> bool {
    let resolution = resolve_link(path);
    let changes = {
        let Some(group) = group.upgrade() else {
            return false;
        };
        let watches = lock(&group);
        if !renamed && watches.followed.resolution == resolution {
            return true;
        }
        watches.changes.clone()
    };
    let is_dir = with_contents && fs::metadata(path).is_ok_and(|meta| meta.is_dir());
    let (followed, failure) = follow(path, resolution, is_dir, &changes);
    let Some(group) = group.upgrade() else {
        return false;
    };
    // The replaced watches are dropped after the lock is released.
    let _replaced = mem::replace(&mut lock(&group).followed, followed);
    if let Some(error) = failure {
        on_failure(error);
    }
    true
}

// Reports any change inside the directory. An error from the watcher counts
// as a change too, since a failed read will sort out what happened.
fn watch_contents(path: &Path, changes: Sender<Change>) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if counts_as_change(&result, |_| true) {
            let _ = changes.send(Change::Contents);
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
    is_dir: bool,
    changes: Sender<Change>,
) -> notify::Result<RecommendedWatcher> {
    let name = entry.file_name().map(ToOwned::to_owned);
    let change = if is_dir {
        Change::Name
    } else {
        Change::Contents
    };
    let mut watcher = notify::recommended_watcher(move |result: notify::Result<Event>| {
        if counts_as_change(&result, |event| {
            concerns_entry(event, name.as_deref(), is_dir)
        }) {
            let _ = changes.send(change);
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

fn concerns_entry(event: &Event, name: Option<&OsStr>, is_dir: bool) -> bool {
    (!is_dir || is_dir_entry_change(event.kind)) && names_entry(event, name)
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

// A change inside a directory may also be reported by its parent as a
// modification of the directory itself (its write time). On Windows that
// seems to arrive late, once the directory is next opened, which resolving
// the link before a report or reading it again after one does, so it would
// make every change report twice. Changes inside are for the contents watch.
// Unknown kinds still count, as they might be a removal.
fn is_dir_entry_change(kind: EventKind) -> bool {
    match kind {
        EventKind::Any | EventKind::Create(_) | EventKind::Remove(_) | EventKind::Other => true,
        EventKind::Modify(modify) => matches!(modify, ModifyKind::Name(_)),
        EventKind::Access(_) => false,
    }
}

// Calls `on_burst` once per burst of changes, after they stop for a moment or
// after a while even if they do not, so that a directory written to all the
// time is still refreshed. The thread ends when the senders are dropped or
// `on_burst` returns false.
fn spawn_debouncer(
    changes: mpsc::Receiver<Change>,
    mut on_burst: impl FnMut(bool) -> bool + Send + 'static,
) {
    thread::Builder::new()
        .name("wafu-io-watch".to_owned())
        .spawn(move || {
            while let Ok(change) = changes.recv() {
                let mut renamed = change == Change::Name;
                let first = Instant::now();
                let mut last = first;
                loop {
                    let deadline = (last + DEBOUNCE).min(first + MAX_WAIT);
                    let now = Instant::now();
                    if now >= deadline {
                        break;
                    }
                    match changes.recv_timeout(deadline - now) {
                        Ok(change) => {
                            last = Instant::now();
                            renamed |= change == Change::Name;
                        }
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
                if !on_burst(renamed) {
                    return;
                }
            }
        })
        .expect("the watch thread should start");
}

#[cfg(test)]
pub(crate) mod tests {
    use notify::event::{CreateKind, DataChange, Flag, MetadataKind, RemoveKind, RenameMode};
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn resolve_link_leaves_what_is_not_a_link() {
        let dir = TempDir::new().unwrap();
        let file = dir.path().join("a");
        fs::write(&file, "a").unwrap();

        assert_eq!(resolve_link(dir.path()), Resolution::NotLink);
        assert_eq!(resolve_link(&file), Resolution::NotLink);
        assert_eq!(
            resolve_link(&dir.path().join("missing")),
            Resolution::NotLink
        );
    }

    #[test]
    fn resolve_link_finds_the_directory_a_link_leads_to() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("other").join("real");
        fs::create_dir_all(&real).unwrap();
        let link = dir.path().join("link");
        if !symlink(&real, &link) {
            return;
        }

        assert_eq!(
            resolve_link(&link),
            Resolution::Target(fs::canonicalize(&real).unwrap())
        );
    }

    #[test]
    fn resolve_link_finds_the_file_a_link_leads_to() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join("other")).unwrap();
        let real = dir.path().join("other").join("b.zip");
        fs::write(&real, "b").unwrap();
        let link = dir.path().join("a.zip");
        if !symlink(&real, &link) {
            return;
        }

        assert_eq!(
            resolve_link(&link),
            Resolution::Target(fs::canonicalize(&real).unwrap())
        );
    }

    #[test]
    fn resolve_link_finds_the_final_target_of_a_chain() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("other").join("real");
        fs::create_dir_all(&real).unwrap();
        let middle = dir.path().join("middle");
        let link = dir.path().join("link");
        if !symlink(&real, &middle) || !symlink(&middle, &link) {
            return;
        }

        assert_eq!(
            resolve_link(&link),
            Resolution::Target(fs::canonicalize(&real).unwrap())
        );
    }

    #[cfg(windows)]
    #[test]
    fn resolve_link_finds_the_directory_a_junction_leads_to() {
        let dir = TempDir::new().unwrap();
        let real = dir.path().join("other").join("real");
        fs::create_dir_all(&real).unwrap();
        let link = dir.path().join("link");
        assert!(junction(&real, &link));

        assert_eq!(
            resolve_link(&link),
            Resolution::Target(fs::canonicalize(&real).unwrap())
        );
    }

    #[test]
    fn resolve_link_fails_on_a_broken_link() {
        let dir = TempDir::new().unwrap();
        let link = dir.path().join("link");
        if !symlink(&dir.path().join("missing"), &link) {
            return;
        }

        assert_eq!(resolve_link(&link), Resolution::Failed);
    }

    #[test]
    fn resolve_link_fails_on_a_cycle() {
        let dir = TempDir::new().unwrap();
        let a = dir.path().join("a");
        let b = dir.path().join("b");
        if !symlink(&b, &a) || !symlink(&a, &b) {
            return;
        }

        assert_eq!(resolve_link(&a), Resolution::Failed);
    }

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
    fn a_directory_entry_counts_only_changes_to_its_name() {
        let watched = Some(OsStr::new("Folder"));
        let counted = [
            EventKind::Create(CreateKind::Folder),
            EventKind::Remove(RemoveKind::Folder),
            EventKind::Modify(ModifyKind::Name(RenameMode::From)),
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            EventKind::Modify(ModifyKind::Name(RenameMode::Any)),
            EventKind::Any,
            EventKind::Other,
        ];
        let ignored = [
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime)),
            EventKind::Modify(ModifyKind::Any),
            EventKind::Modify(ModifyKind::Other),
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
        ];
        for kind in counted {
            assert!(
                counts_as_change(&Ok(event_on(kind, "Folder")), |event| concerns_entry(
                    event, watched, true
                )),
                "{kind:?}"
            );
        }
        for kind in ignored {
            assert!(
                !counts_as_change(&Ok(event_on(kind, "Folder")), |event| concerns_entry(
                    event, watched, true
                )),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_directory_entry_counts_an_error_or_a_rescan() {
        let watched = Some(OsStr::new("Folder"));
        let error = Err(notify::Error::generic("queue overflow"));
        let rescan = Ok(Event::new(EventKind::Other).set_flag(Flag::Rescan));
        assert!(counts_as_change(&error, |event| concerns_entry(
            event, watched, true
        )));
        assert!(counts_as_change(&rescan, |event| concerns_entry(
            event, watched, true
        )));
    }

    #[test]
    fn a_file_entry_counts_any_change_to_it() {
        let watched = Some(OsStr::new("Archive.zip"));
        let counted = [
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            EventKind::Modify(ModifyKind::Data(DataChange::Any)),
            EventKind::Modify(ModifyKind::Metadata(MetadataKind::Any)),
            EventKind::Modify(ModifyKind::Any),
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            EventKind::Access(AccessKind::Close(AccessMode::Write)),
        ];
        for kind in counted {
            assert!(
                counts_as_change(&Ok(event_on(kind, "Archive.zip")), |event| concerns_entry(
                    event, watched, false
                )),
                "{kind:?}"
            );
        }
        assert!(!counts_as_change(
            &Ok(event_on(
                EventKind::Modify(ModifyKind::Data(DataChange::Any)),
                "Other.zip"
            )),
            |event| concerns_entry(event, watched, false)
        ));
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
        spawn_debouncer(receiver, move |_| {
            let _ = reports.send(());
            true
        });

        let start = Instant::now();
        while reported.try_recv().is_err() {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "no report while changes kept coming"
            );
            changes.send(Change::Contents).unwrap();
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn event_on(kind: EventKind, name: &str) -> Event {
        Event::new(kind).add_path(PathBuf::from("dir").join(name))
    }

    #[cfg(unix)]
    pub(crate) fn symlink(target: &Path, link: &Path) -> bool {
        std::os::unix::fs::symlink(target, link).unwrap();
        true
    }

    // Creating a symbolic link takes a privilege on Windows, so a test that
    // needs one is skipped when this returns false.
    #[cfg(windows)]
    pub(crate) fn symlink(target: &Path, link: &Path) -> bool {
        if target.is_dir() {
            std::os::windows::fs::symlink_dir(target, link).is_ok()
        } else {
            std::os::windows::fs::symlink_file(target, link).is_ok()
        }
    }

    #[cfg(windows)]
    pub(crate) fn junction(target: &Path, link: &Path) -> bool {
        std::process::Command::new("cmd")
            .arg("/C")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .output()
            .is_ok_and(|output| output.status.success())
    }
}
