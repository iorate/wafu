use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::{App, Effect, Os, ReadError, Resolved, Side, SpawnName};
use crate::command::Command;
use crate::entry::{Directory, Entry, EntryKind};
use crate::error::{AppError, IoError};
use crate::history::Step;
use crate::location::{ArchiveFormat, Location, Passwords};
use crate::pane::{DrivePurpose, HistoryMove, InFlight, Pane, RequestId};
use crate::path::{drive_letter, normalize, root_of};
use crate::popup::{InputKind, InputPopup, PasswordPurpose, PendingNavigation, Popup};
use crate::preview::is_image_name;

impl App {
    pub(super) fn navigating(&self) -> bool {
        self.panes.iter().any(|pane| {
            matches!(
                pane.loading(),
                Some(
                    InFlight::Resolve { .. }
                        | InFlight::Navigate { .. }
                        | InFlight::Drives { .. }
                        | InFlight::Browse { .. }
                )
            )
        })
    }

    pub(super) fn navigation_target(&self) -> Option<String> {
        match self.earliest_navigation()?.1 {
            InFlight::Resolve { path, .. } => Some(path.display().to_string()),
            InFlight::Navigate { target, .. } => Some(target.to_string()),
            InFlight::Refresh { .. } | InFlight::Drives { .. } | InFlight::Browse { .. } => None,
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
        let target = Location {
            base: normalize(&target.base),
            ..target
        };
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
            fallback: false,
        }));
    }

    pub(super) fn resolve(
        &mut self,
        side: Side,
        path: &Path,
        climb: bool,
        failed: Option<IoError>,
        effects: &mut Vec<Effect>,
    ) {
        let path = normalize(path);
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
            Some(
                InFlight::Navigate { .. }
                    | InFlight::Resolve { .. }
                    | InFlight::Drives { .. }
                    | InFlight::Browse { .. }
            )
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
        if matches!(in_flight, InFlight::Drives { .. } | InFlight::Browse { .. }) {
            self.stop_waiting(side, effects);
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
    ) -> bool {
        let pane = &mut self.panes[side.index()];
        if pane.loading().map(InFlight::request) != Some(request) {
            return false;
        }
        let replaced = match pane.take_loading() {
            Some(InFlight::Navigate {
                target,
                cursor,
                history,
                passwords,
                fallback,
                ..
            }) => match result {
                Ok(directory) => {
                    pane.enter(target, cursor, directory, history, passwords);
                    true
                }
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
                    return false;
                }
                Err(ReadError::Failed(source)) if fallback => {
                    self.log.push(
                        AppError::ReadDir {
                            path: archive_shown(&target),
                            source: source.clone(),
                        }
                        .to_string(),
                    );
                    pane.set_reload_pending(false);
                    // Home is the last resort: looking above it could come
                    // back to it, and reading the old place again would start
                    // the search over.
                    if target.base == normalize(&self.home) {
                        Self::rewatch(side, pane, effects);
                    } else if let Some(parent) = target.base.parent() {
                        self.resolve(side, parent, true, Some(source), effects);
                    } else {
                        self.fall_back_home(side, target.base, effects);
                    }
                    return false;
                }
                Err(ReadError::Failed(source)) => {
                    self.log.push(
                        AppError::ReadDir {
                            path: archive_shown(&target),
                            source,
                        }
                        .to_string(),
                    );
                    if let HistoryMove::Position {
                        pos,
                        step: Some(step),
                    } = history
                        && self.step_history(side, pos, step, effects)
                    {
                        return false;
                    }
                    let pane = &mut self.panes[side.index()];
                    Self::rewatch(side, pane, effects);
                    pane.set_reload_pending(false);
                    self.refresh(side, effects);
                    return false;
                }
            },
            Some(InFlight::Refresh { passwords, .. }) => match result {
                Ok(directory) => {
                    pane.set_directory(directory);
                    pane.set_archive_passwords(passwords);
                    true
                }
                Err(ReadError::PasswordRequired { level, retry }) => {
                    self.ask_listing_password(side, None, passwords, level, retry);
                    false
                }
                Err(ReadError::Failed(source)) => {
                    // The place may be gone; let the I/O layer find out.
                    let path = pane.location().base.clone();
                    self.resolve(side, &path, true, Some(source), effects);
                    return false;
                }
            },
            Some(InFlight::Resolve { .. } | InFlight::Drives { .. } | InFlight::Browse { .. })
            | None => return false,
        };
        let pane = &mut self.panes[side.index()];
        if pane.reload_pending() {
            pane.set_reload_pending(false);
            self.refresh(side, effects);
        }
        replaced
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
        let archive = location.level_file_display(level);
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
        let fallback = failed.is_some() || (climb && pane.location().base.as_os_str().is_empty());
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
                if fallback {
                    self.fall_back_to(side, dir, file, effects);
                } else {
                    self.navigate(side, Location::new(dir), file, HistoryMove::Visit, effects);
                }
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
            None => self.fall_back_home(side, path, effects),
        }
    }

    fn fall_back_to(
        &mut self,
        side: Side,
        dir: PathBuf,
        cursor: Option<OsString>,
        effects: &mut Vec<Effect>,
    ) {
        self.navigate(
            side,
            Location::new(dir),
            cursor,
            HistoryMove::Visit,
            effects,
        );
        if let Some(InFlight::Navigate { fallback, .. }) = self.panes[side.index()].loading_mut() {
            *fallback = true;
        }
    }

    fn fall_back_home(&mut self, side: Side, path: PathBuf, effects: &mut Vec<Effect>) {
        let home = self.home.clone();
        self.log.push(
            AppError::Fallback {
                path,
                fallback: home.clone(),
            }
            .to_string(),
        );
        self.fall_back_to(side, home, None, effects);
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
            Some(InFlight::Drives { .. } | InFlight::Browse { .. }) => {
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
        let entry = pane.cursor_entry();
        if let Some(entry) = entry
            && let Some(error) = self.bad_target("open", &location, [entry.name.as_os_str()])
        {
            self.log.push(error.to_string());
            return;
        }
        if let Some(error) = link_in_archive("open", &location, entry) {
            self.log.push(error.to_string());
            return;
        }
        match entry {
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
        let alternative = &self.config.open_alternative;
        let target = self
            .active_pane()
            .cursor_target()
            .and_then(|(location, entry)| {
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
                Some((line?.clone(), location, entry))
            });
        match target {
            Some((line, location, entry)) => {
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
        if let Some(error) = self.bad_target("open", &location, [entry.name.as_os_str()]) {
            self.log.push(error.to_string());
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
        if let Some(error) = link_in_archive("open", &location, [&entry]) {
            self.log.push(error.to_string());
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
        if let Some(error) = link_in_archive("open", &location, [&entry]) {
            self.log.push(error.to_string());
            return;
        }
        if let Some(error) = self.bad_target("open", &location, [entry.name.as_os_str()]) {
            self.log.push(error.to_string());
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
        let pos = self.active_pane().history().pos();
        self.step_history(self.active, pos, Step::Back, effects);
    }

    pub(super) fn history_forward(&mut self, effects: &mut Vec<Effect>) {
        let pos = self.active_pane().history().pos();
        self.step_history(self.active, pos, Step::Forward, effects);
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
                HistoryMove::Position { pos, step: None },
                effects,
            );
        }
    }

    fn step_history(
        &mut self,
        side: Side,
        from: usize,
        step: Step,
        effects: &mut Vec<Effect>,
    ) -> bool {
        let Some((pos, path)) = self.panes[side.index()].history().beside(from, step) else {
            return false;
        };
        let target = Location::new(path.to_path_buf());
        self.navigate(
            side,
            target,
            None,
            HistoryMove::Position {
                pos,
                step: Some(step),
            },
            effects,
        );
        true
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
        Self::cancel_loading_keeping_reload(&mut self.panes[side.index()], effects);
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
            self.stop_waiting(side, effects);
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
            _ => self.stop_waiting(side, effects),
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
        self.resolve(self.active, &path, true, None, effects);
    }

    pub(super) fn drive_of_active(&self) -> Option<char> {
        drive_letter(&self.active_pane().location().base)
    }

    // Not every OS keeps input from the window while its dialog is open, so
    // the wait holds input off as a move does, and the pane that asked is
    // still the active one when the answer arrives.
    pub(super) fn browse(&mut self, effects: &mut Vec<Effect>) {
        let side = self.active;
        let pane = &mut self.panes[side.index()];
        let dir = pane.location().dir().to_path_buf();
        Self::cancel_loading_keeping_reload(pane, effects);
        let request = self.next_request_id();
        effects.push(Effect::PickDirectory { dir, request });
        self.panes[side.index()].set_loading(Some(InFlight::Browse { request }));
    }

    pub(super) fn directory_picked(
        &mut self,
        request: RequestId,
        path: Option<PathBuf>,
        effects: &mut Vec<Effect>,
    ) {
        let side = self.active;
        if self.active_pane().loading() != Some(&InFlight::Browse { request }) {
            return;
        }
        match path {
            Some(path) => self.resolve(side, &path, false, None, effects),
            None => self.stop_waiting(side, effects),
        }
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
                    | InFlight::Drives { .. }
                    | InFlight::Browse { .. }),
                ) => Some((side, in_flight)),
                _ => None,
            })
            .min_by_key(|(_, in_flight)| in_flight.request())
    }

    fn cancel_loading(pane: &mut Pane, effects: &mut Vec<Effect>) {
        match pane.take_loading() {
            Some(InFlight::Drives { .. } | InFlight::Browse { .. }) | None => {}
            Some(in_flight) => effects.push(Effect::CancelRead {
                request: in_flight.request(),
            }),
        }
        pane.set_reload_pending(false);
    }

    fn cancel_loading_keeping_reload(pane: &mut Pane, effects: &mut Vec<Effect>) {
        let reload = pane.reload_pending() || pane.loading().is_some();
        Self::cancel_loading(pane, effects);
        pane.set_reload_pending(reload);
    }

    fn stop_waiting(&mut self, side: Side, effects: &mut Vec<Effect>) {
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
    location.level_file_display(location.levels().saturating_sub(1))
}

pub(super) fn is_link_in_archive(location: &Location, kind: EntryKind) -> bool {
    location.in_archive() && matches!(kind, EntryKind::Symlink { .. })
}

pub(super) fn link_in_archive<'a>(
    verb: &'static str,
    location: &Location,
    entries: impl IntoIterator<Item = &'a Entry>,
) -> Option<AppError> {
    let entry = entries
        .into_iter()
        .find(|entry| is_link_in_archive(location, entry.kind))?;
    Some(AppError::LinkInArchive {
        verb,
        path: PathBuf::from(location.child_display(&entry.name)),
    })
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
