use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::{App, Effect, Os, ReadError, Resolved, Side, SpawnName};
use crate::command::Command;
use crate::entry::{Directory, EntryKind};
use crate::error::{AppError, IoError};
use crate::location::{ArchiveFormat, Location, Passwords};
use crate::pane::{DrivePurpose, HistoryMove, InFlight, Pane, RequestId};
use crate::path::{drive_letter, root_of};
use crate::popup::{InputKind, InputPopup, PasswordPurpose, PendingNavigation, Popup};
use crate::preview::is_image_name;

impl App {
    pub(super) fn navigating(&self) -> bool {
        self.panes.iter().any(|pane| {
            matches!(
                pane.loading(),
                Some(
                    InFlight::Resolve { .. } | InFlight::Navigate { .. } | InFlight::Drives { .. }
                )
            )
        })
    }

    pub(super) fn navigation_target(&self) -> Option<String> {
        match self.earliest_navigation()?.1 {
            InFlight::Resolve { path, .. } => Some(path.display().to_string()),
            InFlight::Navigate { target, .. } => Some(target.to_string()),
            InFlight::Refresh { .. } | InFlight::Drives { .. } => None,
        }
    }

    pub(super) fn listing_drives(&self) -> bool {
        matches!(
            self.earliest_navigation(),
            Some((_, InFlight::Drives { .. }))
        )
    }

    // The pane keeps showing where it is until the new listing arrives, so a
    // failed or cancelled move leaves it where it was.
    pub(super) fn navigate(
        &mut self,
        side: Side,
        target: Location,
        cursor: Option<OsString>,
        history: HistoryMove,
        effects: &mut Vec<Effect>,
    ) {
        let pane = &self.panes[side.index()];
        let passwords = pane
            .archive_passwords()
            .truncated(target.shared_levels(pane.location()));
        self.navigate_with(side, target, cursor, history, passwords, effects);
    }

    fn navigate_with(
        &mut self,
        side: Side,
        target: Location,
        cursor: Option<OsString>,
        history: HistoryMove,
        passwords: Passwords,
        effects: &mut Vec<Effect>,
    ) {
        Self::cancel_loading(&mut self.panes[side.index()], effects);
        // Watching before reading means a change during the read is not missed.
        effects.push(Effect::Watch {
            side,
            path: target.base.clone(),
        });
        let request = self.next_request_id();
        effects.push(Effect::ReadDir {
            side,
            location: target.clone(),
            passwords: passwords.clone(),
            request,
        });
        self.panes[side.index()].set_loading(Some(InFlight::Navigate {
            request,
            target,
            cursor,
            history,
            passwords,
        }));
    }

    pub(super) fn resolve(
        &mut self,
        side: Side,
        path: PathBuf,
        climb: bool,
        failed: Option<IoError>,
        effects: &mut Vec<Effect>,
    ) {
        Self::cancel_loading(&mut self.panes[side.index()], effects);
        let request = self.next_request_id();
        effects.push(Effect::Resolve {
            side,
            path: path.clone(),
            climb,
            request,
        });
        self.panes[side.index()].set_loading(Some(InFlight::Resolve {
            request,
            path,
            climb,
            failed,
        }));
    }

    // Watching again recovers from changes the watch does not follow, such as
    // a link replaced partway along the path.
    pub(super) fn reload(&mut self, side: Side, effects: &mut Vec<Effect>) {
        let pane = &self.panes[side.index()];
        if !matches!(
            pane.loading(),
            Some(InFlight::Navigate { .. } | InFlight::Resolve { .. } | InFlight::Drives { .. })
        ) {
            Self::rewatch(side, pane, effects);
        }
        self.refresh(side, effects);
    }

    pub(super) fn refresh(&mut self, side: Side, effects: &mut Vec<Effect>) {
        let pane = &mut self.panes[side.index()];
        if pane.loading().is_some() {
            pane.set_reload_pending(true);
            return;
        }
        let passwords = pane.archive_passwords().clone();
        self.refresh_with(side, passwords, effects);
    }

    fn refresh_with(&mut self, side: Side, passwords: Passwords, effects: &mut Vec<Effect>) {
        let location = self.panes[side.index()].location().clone();
        if location.base.as_os_str().is_empty() {
            return;
        }
        let request = self.next_request_id();
        effects.push(Effect::ReadDir {
            side,
            location,
            passwords: passwords.clone(),
            request,
        });
        self.panes[side.index()].set_loading(Some(InFlight::Refresh { request, passwords }));
    }

    pub(super) fn cancel_navigation(&mut self, effects: &mut Vec<Effect>) {
        let Some((side, in_flight)) = self.earliest_navigation() else {
            return;
        };
        if matches!(in_flight, InFlight::Drives { .. }) {
            self.stop_listing_drives(side, effects);
            return;
        }
        let pane = &mut self.panes[side.index()];
        Self::cancel_loading(pane, effects);
        Self::rewatch(side, pane, effects);
        self.refresh(side, effects);
    }

    pub(super) fn dir_loaded(
        &mut self,
        side: Side,
        request: RequestId,
        result: Result<Directory, ReadError>,
        effects: &mut Vec<Effect>,
    ) {
        let pane = &mut self.panes[side.index()];
        if pane.loading().map(InFlight::request) != Some(request) {
            return;
        }
        match pane.take_loading() {
            Some(InFlight::Navigate {
                target,
                cursor,
                history,
                passwords,
                ..
            }) => match result {
                Ok(directory) => pane.enter(target, cursor, directory, history, passwords),
                Err(ReadError::PasswordRequired { level, retry }) => {
                    Self::rewatch(side, pane, effects);
                    pane.set_reload_pending(false);
                    self.refresh(side, effects);
                    let navigation = PendingNavigation {
                        target,
                        cursor,
                        history,
                    };
                    self.ask_listing_password(side, Some(navigation), passwords, level, retry);
                    return;
                }
                Err(ReadError::Failed(source)) => {
                    self.log.push(
                        AppError::ReadDir {
                            path: archive_shown(&target),
                            source,
                        }
                        .to_string(),
                    );
                    Self::rewatch(side, pane, effects);
                    pane.set_reload_pending(false);
                    self.refresh(side, effects);
                    return;
                }
            },
            Some(InFlight::Refresh { passwords, .. }) => match result {
                Ok(directory) => {
                    pane.set_directory(directory);
                    pane.set_archive_passwords(passwords);
                }
                Err(ReadError::PasswordRequired { level, retry }) => {
                    self.ask_listing_password(side, None, passwords, level, retry);
                }
                Err(ReadError::Failed(source)) => {
                    // The place may be gone; let the I/O layer find out.
                    let path = pane.location().base.clone();
                    self.resolve(side, path, true, Some(source), effects);
                    return;
                }
            },
            Some(InFlight::Resolve { .. } | InFlight::Drives { .. }) | None => return,
        }
        let pane = &mut self.panes[side.index()];
        if pane.reload_pending() {
            pane.set_reload_pending(false);
            self.refresh(side, effects);
        }
    }

    // `passwords` are those the read was made with. A wrong password the
    // pane remembered is forgotten. While an operation runs or another popup
    // is open, no prompt can be shown, so the read fails and the pane stays
    // where it is.
    fn ask_listing_password(
        &mut self,
        side: Side,
        navigation: Option<PendingNavigation>,
        passwords: Passwords,
        level: usize,
        retry: bool,
    ) {
        let pane = &mut self.panes[side.index()];
        let location = navigation
            .as_ref()
            .map_or_else(|| pane.location().clone(), |n| n.target.clone());
        if retry && level < location.shared_levels(pane.location()) {
            pane.set_archive_password(level, None);
        }
        let archive = level_shown(&location, level);
        if self.operation.is_some() || self.popup.is_some() {
            let reason = if retry {
                "wrong password"
            } else {
                "a password is required"
            };
            self.log.push(
                AppError::ReadDir {
                    path: archive,
                    source: IoError::new(reason),
                }
                .to_string(),
            );
            return;
        }
        self.popup = Some(Popup::Input(InputPopup::password(
            archive,
            level,
            retry,
            PasswordPurpose::Listing {
                side,
                passwords,
                navigation: navigation.map(Box::new),
            },
        )));
    }

    // The read is made again with the password, which the pane takes once
    // the listing arrives.
    pub(super) fn listing_password(&mut self, popup: InputPopup, effects: &mut Vec<Effect>) {
        let password = popup.text(0).to_owned();
        if password.is_empty() {
            self.popup = Some(Popup::Input(popup));
            return;
        }
        let InputKind::ArchivePassword {
            level,
            purpose:
                PasswordPurpose::Listing {
                    side,
                    mut passwords,
                    navigation,
                },
            ..
        } = popup.kind
        else {
            return;
        };
        passwords.set(level, Some(password));
        if let Some(navigation) = navigation {
            let PendingNavigation {
                target,
                cursor,
                history,
            } = *navigation;
            self.navigate_with(side, target, cursor, history, passwords, effects);
        } else {
            Self::cancel_loading(&mut self.panes[side.index()], effects);
            self.refresh_with(side, passwords, effects);
        }
    }

    pub(super) fn resolved(
        &mut self,
        side: Side,
        request: RequestId,
        result: Option<Resolved>,
        effects: &mut Vec<Effect>,
    ) {
        let pane = &mut self.panes[side.index()];
        if pane.loading().map(InFlight::request) != Some(request) {
            return;
        }
        let Some(InFlight::Resolve {
            path,
            climb,
            failed,
            ..
        }) = pane.take_loading()
        else {
            return;
        };
        match result {
            Some(Resolved { dir, file })
                if climb && still_here(pane.location(), &dir, file.as_deref()) =>
            {
                // Still here, so the read error was the whole story. Reading again
                // would fail the same way and resolve again without end.
                if let Some(source) = failed {
                    self.log
                        .push(AppError::ReadDir { path, source }.to_string());
                }
                Self::rewatch(side, pane, effects);
            }
            Some(Resolved { dir, file }) => {
                if file.is_none() && dir != path {
                    self.log.push(
                        AppError::Fallback {
                            path,
                            fallback: dir.clone(),
                        }
                        .to_string(),
                    );
                }
                self.navigate(side, Location::new(dir), file, HistoryMove::Visit, effects);
            }
            None if !climb => {
                self.log.push(AppError::NotFound { path }.to_string());
                Self::rewatch(side, pane, effects);
                self.refresh(side, effects);
            }
            // Home is only for a pane with nowhere else to be; a pane that is
            // somewhere and fails to move elsewhere stays there.
            None if failed.is_none() && !pane.location().base.as_os_str().is_empty() => {
                self.log.push(AppError::Unavailable { path }.to_string());
                Self::rewatch(side, pane, effects);
                self.refresh(side, effects);
            }
            None => {
                let home = self.home.clone();
                self.log.push(
                    AppError::Fallback {
                        path,
                        fallback: home.clone(),
                    }
                    .to_string(),
                );
                self.navigate(side, Location::new(home), None, HistoryMove::Visit, effects);
            }
        }
    }

    pub(super) fn location_changed(&mut self, side: Side, path: &Path, effects: &mut Vec<Effect>) {
        let pane = &mut self.panes[side.index()];
        match pane.loading() {
            Some(InFlight::Navigate { target, .. }) => {
                if target.base == path {
                    pane.set_reload_pending(true);
                }
            }
            Some(InFlight::Resolve { .. }) => {}
            Some(InFlight::Drives { .. }) => {
                if pane.location().base == path {
                    pane.set_reload_pending(true);
                }
            }
            Some(InFlight::Refresh { .. }) | None => {
                if pane.location().base == path {
                    self.refresh(side, effects);
                }
            }
        }
    }

    pub(super) fn open_entry(&mut self, effects: &mut Vec<Effect>) {
        let side = self.active;
        let pane = &self.panes[side.index()];
        let location = pane.location().clone();
        match pane.cursor_entry() {
            // `..` stands for this place, and opening it would only show it again, so it moves up.
            None => self.go_parent(effects),
            Some(entry) if entry.kind.is_dir_like() => {
                let target = if location.in_archive() {
                    location.child(&entry.name.to_string_lossy())
                } else {
                    Location::new(location.base.join(&entry.name))
                };
                self.navigate(side, target, None, HistoryMove::Visit, effects);
            }
            Some(entry) if entry.kind.is_broken_link() => {
                let path = location.base.join(&entry.name);
                self.log.push(AppError::BrokenLink { path }.to_string());
            }
            Some(entry) if entry.kind.is_special() => {
                let path = location.base.join(&entry.name);
                self.log.push(AppError::NotRegularFile { path }.to_string());
            }
            // A link inside an archive leads nowhere until extracted.
            Some(entry) if is_link_in_archive(&location, entry.kind) => {}
            Some(entry) => {
                if let Some(format) = ArchiveFormat::from_name(&entry.name) {
                    let name = entry.name.clone();
                    self.enter_archive(&location, &name, Some(format), effects);
                // The name decides first: where a file system marks every file
                // executable (a Windows drive mounted in WSL), the bit tells
                // nothing, and an image should still be previewed.
                } else if entry.kind.is_executable() && !is_image_name(&entry.name) {
                    self.run_file(effects);
                } else {
                    let entry = entry.clone();
                    self.open_preview(&entry, effects);
                }
            }
        }
    }

    pub(super) fn open_alternative(&mut self, effects: &mut Vec<Effect>) {
        let pane = self.active_pane();
        let location = pane.location().clone();
        let Some(entry) = pane.cursor_entry().cloned() else {
            self.open_entry(effects);
            return;
        };
        let alternative = &self.config.open_alternative;
        let line = if entry.kind.is_dir_like() {
            alternative.dir.as_ref()
        } else if entry.kind.is_broken_link()
            || entry.kind.is_special()
            || is_link_in_archive(&location, entry.kind)
        {
            None
        } else {
            let (_, extension) = entry.split_name();
            alternative.extensions.get(&extension.to_ascii_lowercase())
        };
        match line.cloned() {
            Some(line) => {
                let name = SpawnName::Command(Command::OpenAlternative);
                self.start_on_entry(name, line, &location, &entry, effects);
            }
            None => self.open_entry(effects),
        }
    }

    pub(super) fn open_as_text(&mut self, effects: &mut Vec<Effect>) {
        let Some((location, entry)) = self.active_pane().cursor_target() else {
            return;
        };
        if entry.kind.is_dir_like() {
            return;
        }
        if entry.kind.is_broken_link() {
            let path = location.base.join(&entry.name);
            self.log.push(AppError::BrokenLink { path }.to_string());
            return;
        }
        if entry.kind.is_special() {
            let path = location.base.join(&entry.name);
            self.log.push(AppError::NotRegularFile { path }.to_string());
            return;
        }
        // A link inside an archive leads nowhere until extracted.
        if is_link_in_archive(&location, entry.kind) {
            return;
        }
        self.open_document(&location, &entry.name, effects);
    }

    pub(super) fn enter_as_archive(&mut self, effects: &mut Vec<Effect>) {
        let Some((location, entry)) = self.active_pane().cursor_target() else {
            return;
        };
        if entry.kind.is_dir_like() {
            return;
        }
        if entry.kind.is_special() {
            let path = location.base.join(&entry.name);
            self.log.push(AppError::NotRegularFile { path }.to_string());
            return;
        }
        // A `..` row that is not a directory is the archive whose root the
        // pane is at; entering it would only reload it, dropping the marks.
        if self.active_pane().cursor_entry().is_none() {
            return;
        }
        // A link inside an archive leads nowhere until extracted.
        if is_link_in_archive(&location, entry.kind) {
            return;
        }
        // An unknown extension leaves the format to the content.
        let format = ArchiveFormat::from_name(&entry.name);
        self.enter_archive(&location, &entry.name, format, effects);
    }

    fn enter_archive(
        &mut self,
        location: &Location,
        name: &OsStr,
        format: Option<ArchiveFormat>,
        effects: &mut Vec<Effect>,
    ) {
        let target = if location.in_archive() {
            location.nested_root(&name.to_string_lossy(), format)
        } else {
            Location::archive_root(location.base.join(name), format)
        };
        self.navigate(self.active, target, None, HistoryMove::Visit, effects);
    }

    pub(super) fn go_parent(&mut self, effects: &mut Vec<Effect>) {
        let side = self.active;
        let pane = &self.panes[side.index()];
        if let Some(parent) = pane.location().parent() {
            let came_from = pane.location().leaf_name();
            self.navigate(side, parent, came_from, HistoryMove::Visit, effects);
        }
    }

    pub(super) fn go_root(&mut self, effects: &mut Vec<Effect>) {
        let location = self.active_pane().location();
        let target = match location.innermost_root() {
            Some(root) => root,
            None => match root_of(&location.base) {
                Some(root) => Location::new(root),
                None => return,
            },
        };
        self.navigate(self.active, target, None, HistoryMove::Visit, effects);
    }

    pub(super) fn go_home(&mut self, effects: &mut Vec<Effect>) {
        let home = self.home.clone();
        self.navigate(
            self.active,
            Location::new(home),
            None,
            HistoryMove::Visit,
            effects,
        );
    }

    pub(super) fn history_back(&mut self, effects: &mut Vec<Effect>) {
        let target = self.active_pane().history().back();
        self.go_to_history(target.map(|(pos, path)| (pos, path.to_path_buf())), effects);
    }

    pub(super) fn history_forward(&mut self, effects: &mut Vec<Effect>) {
        let target = self.active_pane().history().forward();
        self.go_to_history(target.map(|(pos, path)| (pos, path.to_path_buf())), effects);
    }

    pub(super) fn go_to_history(
        &mut self,
        target: Option<(usize, PathBuf)>,
        effects: &mut Vec<Effect>,
    ) {
        if let Some((pos, path)) = target {
            self.navigate(
                self.active,
                Location::new(path),
                None,
                HistoryMove::Position(pos),
                effects,
            );
        }
    }

    pub(super) fn sync_to_other(&mut self, effects: &mut Vec<Effect>) {
        self.copy_location(self.active.other(), self.active, effects);
    }

    pub(super) fn sync_other_to_this(&mut self, effects: &mut Vec<Effect>) {
        self.copy_location(self.active, self.active.other(), effects);
    }

    pub(super) fn swap_panes(&mut self, effects: &mut Vec<Effect>) {
        let (left, right) = (&self.panes[0], &self.panes[1]);
        if left.location().base.as_os_str().is_empty()
            || right.location().base.as_os_str().is_empty()
        {
            return;
        }
        let left = (left.location().clone(), left.archive_passwords().clone());
        let right = (right.location().clone(), right.archive_passwords().clone());
        self.navigate_with(
            Side::Left,
            right.0,
            None,
            HistoryMove::Visit,
            right.1,
            effects,
        );
        self.navigate_with(
            Side::Right,
            left.0,
            None,
            HistoryMove::Visit,
            left.1,
            effects,
        );
    }

    // The passwords go along with the location.
    fn copy_location(&mut self, from: Side, to: Side, effects: &mut Vec<Effect>) {
        let pane = &self.panes[from.index()];
        let target = pane.location().clone();
        if target.base.as_os_str().is_empty() {
            return;
        }
        let passwords = pane.archive_passwords().clone();
        self.navigate_with(to, target, None, HistoryMove::Visit, passwords, effects);
    }

    // The request takes the active pane's load slot: input is held off while
    // a pane loads, so the pane that asked is still the active one when the
    // list arrives.
    pub(super) fn list_drives(
        &mut self,
        purpose: DrivePurpose,
        command: Command,
        effects: &mut Vec<Effect>,
    ) {
        if self.os != Os::Windows {
            self.log.push(AppError::WindowsOnly { command }.to_string());
            return;
        }
        let side = self.active;
        let pane = &mut self.panes[side.index()];
        let reload = pane.reload_pending() || pane.loading().is_some();
        Self::cancel_loading(pane, effects);
        pane.set_reload_pending(reload);
        let request = self.next_request_id();
        effects.push(Effect::ListDrives { request });
        self.panes[side.index()].set_loading(Some(InFlight::Drives { request, purpose }));
    }

    pub(super) fn drives(
        &mut self,
        request: RequestId,
        mut letters: Vec<char>,
        effects: &mut Vec<Effect>,
    ) {
        let side = self.active;
        let purpose = match self.active_pane().loading() {
            Some(&InFlight::Drives {
                request: pending,
                purpose,
            }) if pending == request => purpose,
            _ => return,
        };
        letters.sort_unstable();
        letters.dedup();
        if purpose == DrivePurpose::Show {
            self.stop_listing_drives(side, effects);
            self.open_drives(&letters);
            return;
        }
        let target = (!letters.is_empty()).then(|| {
            let current = drive_letter(&self.active_pane().location().base)
                .and_then(|letter| letters.iter().position(|&l| l == letter));
            let last = letters.len() - 1;
            let index = match (purpose, current) {
                (DrivePurpose::Prev, Some(0) | None) => last,
                (DrivePurpose::Prev, Some(index)) => index - 1,
                (_, Some(index)) if index == last => 0,
                (_, Some(index)) => index + 1,
                (_, None) => 0,
            };
            letters[index]
        });
        match target {
            Some(letter) if self.drive_of_active() != Some(letter) => {
                self.switch_drive(letter, effects);
            }
            _ => self.stop_listing_drives(side, effects),
        }
    }

    pub(super) fn switch_drive(&mut self, letter: char, effects: &mut Vec<Effect>) {
        let pane = self.active_pane();
        if drive_letter(&pane.location().base) == Some(letter) {
            return;
        }
        let path = pane
            .drive_memory()
            .get(&letter)
            .cloned()
            .unwrap_or_else(|| PathBuf::from(format!("{letter}:\\")));
        self.resolve(self.active, path, true, None, effects);
    }

    pub(super) fn drive_of_active(&self) -> Option<char> {
        drive_letter(&self.active_pane().location().base)
    }

    pub(super) fn next_request_id(&mut self) -> RequestId {
        let request = RequestId(self.next_request);
        self.next_request += 1;
        request
    }

    fn earliest_navigation(&self) -> Option<(Side, &InFlight)> {
        [Side::Left, Side::Right]
            .into_iter()
            .filter_map(|side| match self.panes[side.index()].loading() {
                Some(
                    in_flight @ (InFlight::Resolve { .. }
                    | InFlight::Navigate { .. }
                    | InFlight::Drives { .. }),
                ) => Some((side, in_flight)),
                _ => None,
            })
            .min_by_key(|(_, in_flight)| in_flight.request())
    }

    fn cancel_loading(pane: &mut Pane, effects: &mut Vec<Effect>) {
        match pane.take_loading() {
            Some(InFlight::Drives { .. }) | None => {}
            Some(in_flight) => effects.push(Effect::CancelRead {
                request: in_flight.request(),
            }),
        }
        pane.set_reload_pending(false);
    }

    fn stop_listing_drives(&mut self, side: Side, effects: &mut Vec<Effect>) {
        let pane = &mut self.panes[side.index()];
        pane.set_loading(None);
        if pane.reload_pending() {
            pane.set_reload_pending(false);
            self.refresh(side, effects);
        }
    }

    // The watch moved to the target before the read; a pane that stays put
    // needs its own place watched again.
    fn rewatch(side: Side, pane: &Pane, effects: &mut Vec<Effect>) {
        let path = &pane.location().base;
        if !path.as_os_str().is_empty() {
            effects.push(Effect::Watch {
                side,
                path: path.clone(),
            });
        }
    }
}

// The file a location stands for in messages: `base`, or for a place inside a
// nested archive that archive, as the user sees its path, since `base` is then
// only the outermost file.
fn archive_shown(location: &Location) -> PathBuf {
    level_shown(location, location.levels().saturating_sub(1))
}

// The archive at `level` of a location, as `archive_shown` names it.
pub(super) fn level_shown(location: &Location, level: usize) -> PathBuf {
    match location.level_root(level) {
        Some(root) if level > 0 => PathBuf::from(root.to_string()),
        _ => location.base.clone(),
    }
}

pub(super) fn is_link_in_archive(location: &Location, kind: EntryKind) -> bool {
    location.in_archive() && matches!(kind, EntryKind::Symlink { .. })
}

// Whether a resolve of the pane's base found it where it is: the directory
// itself, or, inside an archive, the archive file in its directory.
fn still_here(location: &Location, dir: &Path, file: Option<&OsStr>) -> bool {
    if location.in_archive() {
        Some(dir) == location.base.parent() && file == location.base.file_name()
    } else {
        dir == location.base
    }
}
