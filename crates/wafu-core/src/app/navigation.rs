use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::{App, DrivePurpose, Effect, Os, Resolved, Side};
use crate::command::Command;
use crate::entry::{Directory, EntryKind};
use crate::error::{AppError, IoError};
use crate::location::{ArchiveFormat, Location};
use crate::pane::{HistoryMove, InFlight, Pane, RequestId};
use crate::path::{drive_letter, root_of};

impl App {
    pub(super) fn navigating(&self) -> bool {
        self.panes.iter().any(|pane| {
            matches!(
                pane.loading(),
                Some(InFlight::Resolve { .. } | InFlight::Navigate { .. })
            )
        })
    }

    pub(super) fn navigation_targets(&self) -> Vec<String> {
        self.panes
            .iter()
            .filter_map(|pane| match pane.loading() {
                Some(InFlight::Resolve { path, .. }) => Some(path.display().to_string()),
                Some(InFlight::Navigate { target, .. }) => Some(target.to_string()),
                _ => None,
            })
            .collect()
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
        if *self.panes[side.index()].location() == target && cursor.is_none() {
            return;
        }
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
            request,
        });
        self.panes[side.index()].set_loading(Some(InFlight::Navigate {
            request,
            target,
            cursor,
            history,
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

    pub(super) fn refresh(&mut self, side: Side, effects: &mut Vec<Effect>) {
        let pane = &mut self.panes[side.index()];
        if pane.loading().is_some() {
            pane.set_reload_pending(true);
            return;
        }
        let location = pane.location().clone();
        if location.base.as_os_str().is_empty() {
            return;
        }
        let request = self.next_request_id();
        effects.push(Effect::ReadDir {
            side,
            location,
            request,
        });
        self.panes[side.index()].set_loading(Some(InFlight::Refresh { request }));
    }

    pub(super) fn cancel_navigations(&mut self, effects: &mut Vec<Effect>) {
        for side in [Side::Left, Side::Right] {
            let pane = &mut self.panes[side.index()];
            if matches!(
                pane.loading(),
                Some(InFlight::Resolve { .. } | InFlight::Navigate { .. })
            ) {
                Self::cancel_loading(pane, effects);
                Self::rewatch(side, pane, effects);
            }
        }
    }

    pub(super) fn dir_loaded(
        &mut self,
        side: Side,
        request: RequestId,
        result: Result<Directory, IoError>,
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
                ..
            }) => match result {
                Ok(directory) => pane.enter(target, cursor, directory, history),
                Err(source) => {
                    self.log.push(
                        AppError::ReadDir {
                            path: target.base,
                            source,
                        }
                        .to_string(),
                    );
                    Self::rewatch(side, pane, effects);
                }
            },
            Some(InFlight::Refresh { .. }) => match result {
                Ok(directory) => pane.set_directory(directory),
                Err(source) => {
                    // The place may be gone; let the I/O layer find out.
                    let path = pane.location().base.clone();
                    self.resolve(side, path, true, Some(source), effects);
                    return;
                }
            },
            Some(InFlight::Resolve { .. }) | None => return,
        }
        let pane = &mut self.panes[side.index()];
        if pane.reload_pending() {
            pane.set_reload_pending(false);
            self.refresh(side, effects);
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
                // Still here, so the read error was the whole story.
                if let Some(source) = failed {
                    self.log
                        .push(AppError::ReadDir { path, source }.to_string());
                }
                Self::rewatch(side, pane, effects);
            }
            // The pane is already there; only its watch needs restoring.
            Some(Resolved { dir, file: None }) if dir == pane.location().base => {
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
            }
            // Home is only for a pane with nowhere else to be; a pane that is
            // somewhere stays there.
            None if !pane.location().base.as_os_str().is_empty() => {
                self.log.push(AppError::Unavailable { path }.to_string());
                Self::rewatch(side, pane, effects);
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
            Some(entry)
                if location.in_archive() && matches!(entry.kind, EntryKind::Symlink { .. }) => {}
            Some(entry) => {
                if let Some(format) = ArchiveFormat::from_name(&entry.name) {
                    let name = entry.name.clone();
                    self.enter_archive(&name, format, effects);
                } else {
                    let entry = entry.clone();
                    self.open_preview(&entry, effects);
                }
            }
        }
    }

    pub(super) fn enter_as_archive(&mut self, effects: &mut Vec<Effect>) {
        let Some(entry) = self.active_pane().cursor_entry() else {
            return;
        };
        if entry.kind.is_dir_like() {
            return;
        }
        if entry.kind.is_special() {
            let path = self.active_pane().location().base.join(&entry.name);
            self.log.push(AppError::NotRegularFile { path }.to_string());
            return;
        }
        let format = ArchiveFormat::from_name(&entry.name).unwrap_or(ArchiveFormat::Zip);
        let name = entry.name.clone();
        self.enter_archive(&name, format, effects);
    }

    fn enter_archive(&mut self, name: &OsStr, format: ArchiveFormat, effects: &mut Vec<Effect>) {
        let location = self.active_pane().location();
        if location.in_archive() {
            let path = PathBuf::from(location.child_display(name));
            self.log.push(AppError::NestedArchive { path }.to_string());
            return;
        }
        let target = Location::archive_root(location.base.join(name), format);
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
        let target = match &location.archive {
            Some(archive) => Location::archive_root(location.base.clone(), archive.format),
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
        let target = self.panes[self.active.other().index()].location().clone();
        if target.base.as_os_str().is_empty() {
            return;
        }
        self.navigate(self.active, target, None, HistoryMove::Visit, effects);
    }

    pub(super) fn sync_other_to_this(&mut self, effects: &mut Vec<Effect>) {
        let target = self.active_pane().location().clone();
        if target.base.as_os_str().is_empty() {
            return;
        }
        self.navigate(
            self.active.other(),
            target,
            None,
            HistoryMove::Visit,
            effects,
        );
    }

    pub(super) fn swap_panes(&mut self, effects: &mut Vec<Effect>) {
        let left = self.panes[0].location().clone();
        let right = self.panes[1].location().clone();
        if left.base.as_os_str().is_empty() || right.base.as_os_str().is_empty() {
            return;
        }
        self.navigate(Side::Left, right, None, HistoryMove::Visit, effects);
        self.navigate(Side::Right, left, None, HistoryMove::Visit, effects);
    }

    pub(super) fn list_drives(
        &mut self,
        purpose: DrivePurpose,
        command: Command,
        effects: &mut Vec<Effect>,
    ) {
        if self.os == Os::Windows {
            effects.push(Effect::ListDrives { purpose });
        } else {
            self.log.push(AppError::WindowsOnly { command }.to_string());
        }
    }

    pub(super) fn drives(
        &mut self,
        purpose: DrivePurpose,
        mut letters: Vec<char>,
        effects: &mut Vec<Effect>,
    ) {
        letters.sort_unstable();
        letters.dedup();
        match purpose {
            DrivePurpose::Show => self.open_drives(&letters),
            DrivePurpose::Prev | DrivePurpose::Next => {
                if letters.is_empty() {
                    return;
                }
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
                self.switch_drive(letters[index], effects);
            }
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

    fn cancel_loading(pane: &mut Pane, effects: &mut Vec<Effect>) {
        if let Some(in_flight) = pane.take_loading() {
            effects.push(Effect::CancelRead {
                request: in_flight.request(),
            });
        }
        pane.set_reload_pending(false);
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

// Whether a resolve of the pane's base found it where it is: the directory
// itself, or, inside an archive, the archive file in its directory.
fn still_here(location: &Location, dir: &Path, file: Option<&OsStr>) -> bool {
    if location.in_archive() {
        Some(dir) == location.base.parent() && file == location.base.file_name()
    } else {
        dir == location.base
    }
}
