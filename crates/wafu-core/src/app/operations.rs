use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use super::navigation::link_in_archive;
use super::{App, Change, Effect, MenuAnchor, Os, PreviewSource, Side, SpawnName};
use crate::command::{Command, Context};
use crate::entry::{Entry, EntryKind};
use crate::error::{AppError, IoError};
use crate::external::{CommandLine, Expansion, Placeholder};
use crate::filter::Filter;
use crate::key::{Key, KeyBase, NamedKey};
use crate::location::{ArchiveFormat, Location, Passwords};
use crate::operation::{
    ArchiveEntries, ArchiveOutput, FindQuery, OperationEvent, OperationKind, OperationSpec,
    Outcome, OutcomeLine, Renames, Reply, Sources, plural,
};
use crate::pane::{HistoryMove, Targets};
use crate::popup::{
    ChoiceKind, ChoicePopup, ConflictPopup, InputKind, InputPopup, ListPopup, PasswordPurpose,
    Popup, RenameMode, RenamePopup,
};
use crate::text::{NameProblem, name_problem, same_name};
use crate::view::PopupView;

const DEFAULT_ARCHIVE_STEM: &str = "archive";

impl App {
    pub(super) fn run_file_command(&mut self, command: Command, effects: &mut Vec<Effect>) {
        if self.command_location(&command).in_archive() && !allowed_in_archive(&command) {
            self.log.push(AppError::InArchive { command }.to_string());
            return;
        }
        if self.config.filelist.require_marks
            && requires_marks(&command)
            && self.active_pane().marked_entries().next().is_none()
        {
            self.log.push(AppError::NoMarks { command }.to_string());
            return;
        }
        match command {
            Command::OpenWithOs => self.open_with_os(effects),
            Command::Copy => self.start_transfer(OperationKind::Copy, effects),
            Command::Move => self.start_transfer(OperationKind::Move, effects),
            Command::Delete => self.confirm_delete(),
            // No confirmation, unlike delete: what goes to the trash can be
            // restored, and where it cannot go Windows warns and elsewhere the
            // move fails.
            Command::Trash => {
                let Some((location, names)) = self.target_names(OperationKind::Trash.verb()) else {
                    return;
                };
                self.start_removal(OperationKind::Trash, location.base, names, effects);
            }
            Command::Rename => self.open_rename(),
            Command::RenameReplace => self.open_batch_rename(RenameMode::Replace),
            Command::RenameTemplate => self.open_batch_rename(RenameMode::Template),
            Command::NewDir => self.open_input(InputKind::NewDir {
                dir: self.active_pane().location().base.clone(),
            }),
            Command::NewFile => self.open_input(InputKind::NewFile {
                dir: self.active_pane().location().base.clone(),
            }),
            Command::NewText => self.open_input(InputKind::NewText {
                dir: self.active_pane().location().base.clone(),
            }),
            Command::Edit => self.edit(effects),
            Command::CopyPath => self.copy_to_clipboard(true, effects),
            Command::CopyName => self.copy_to_clipboard(false, effects),
            Command::CopyFiles => self.copy_files(effects),
            Command::ShellMenu if self.os != Os::Windows => {
                self.log.push(AppError::WindowsOnly { command }.to_string());
            }
            Command::ShellMenu => {
                let anchor = MenuAnchor::Row {
                    side: self.active,
                    row: self.active_pane().cursor(),
                };
                self.shell_menu(anchor, effects);
            }
            Command::CalculateSize => self.start_calculation(OperationKind::CalculateSize, effects),
            Command::CalculateSizeOnDisk => {
                self.start_calculation(OperationKind::CalculateSizeOnDisk, effects);
            }
            Command::OpenAsArchive => self.open_as_archive(effects),
            Command::Extract => self.start_extract(false, effects),
            Command::ExtractHere => self.start_extract(true, effects),
            Command::Compress => self.open_compress(false),
            Command::CompressHere => self.open_compress(true),
            _ => {}
        }
    }

    pub(super) fn shell_menu(&mut self, anchor: MenuAnchor, effects: &mut Vec<Effect>) {
        let Some(targets) = self.active_pane().targets() else {
            return;
        };
        if targets.location.in_archive() {
            let command = Command::ShellMenu;
            self.log.push(AppError::InArchive { command }.to_string());
            return;
        }
        let names = targets.entries.iter().map(|entry| entry.name.as_os_str());
        if let Some(error) = self.bad_target("show the shell menu for", &targets.location, names) {
            self.log.push(error.to_string());
            return;
        }
        let paths = targets
            .entries
            .iter()
            .map(|entry| targets.location.base.join(&entry.name))
            .collect();
        effects.push(Effect::ShellMenu { paths, anchor });
    }

    fn start_transfer(&mut self, kind: OperationKind, effects: &mut Vec<Effect>) {
        let Some((location, names)) = self.target_names(kind.verb()) else {
            return;
        };
        let Some(dest) = self.other_dest(kind.verb()) else {
            return;
        };
        let source_dir = location.base.clone();
        let sources = if location.in_archive() {
            Sources::Entries(self.archive_entries(&location, &names))
        } else {
            source_paths(&source_dir, &names)
        };
        self.start_operation(kind, source_dir, names, sources, Some(dest), effects);
    }

    fn start_extract(&mut self, here: bool, effects: &mut Vec<Effect>) {
        let kind = OperationKind::Extract {
            into_dir: self.config.filelist.extract_creates_dir,
        };
        let Some((location, names)) = self.target_names(kind.verb()) else {
            return;
        };
        let dest = if here {
            self.active_pane().location().base.clone()
        } else {
            let Some(dest) = self.other_dest(kind.verb()) else {
                return;
            };
            dest
        };
        let source_dir = location.base.clone();
        let sources = if location.in_archive() {
            Sources::Entries(self.archive_entries(&location, &names))
        } else {
            source_paths(&source_dir, &names)
        };
        self.start_operation(kind, source_dir, names, sources, Some(dest), effects);
    }

    fn open_compress(&mut self, here: bool) {
        let Some((location, names)) = self.target_names(OperationKind::Compress.verb()) else {
            return;
        };
        let dest = if here {
            self.active_pane().location().base.clone()
        } else {
            let Some(dest) = self.other_dest(OperationKind::Compress.verb()) else {
                return;
            };
            dest
        };
        let dir = location.base;
        let stem = match names.as_slice() {
            [name] => name.to_string_lossy().into_owned(),
            _ => dir.file_name().map_or_else(
                || DEFAULT_ARCHIVE_STEM.to_owned(),
                |name| name.to_string_lossy().into_owned(),
            ),
        };
        let initial = format!("{stem}.{}", ArchiveFormat::Zip.extension());
        self.popup = Some(Popup::Input(InputPopup::compress(
            dir, dest, names, here, initial,
        )));
    }

    pub(super) fn confirm_compress(
        &mut self,
        dir: PathBuf,
        dest: PathBuf,
        names: Vec<OsString>,
        name: &str,
        password: &str,
        effects: &mut Vec<Effect>,
    ) {
        if name.is_empty() {
            return;
        }
        if let Some(problem) = name_problem(name, self.os) {
            let error = AppError::BadArchiveName {
                path: dest.join(name),
                reason: problem.reason(),
            };
            self.log.push(error.to_string());
            return;
        }
        let Some(format) = ArchiveFormat::from_name(OsStr::new(name)) else {
            let path = dest.join(name);
            self.log.push(AppError::NoFormat { path }.to_string());
            return;
        };
        if !password.is_empty() && !format.takes_password() {
            let path = dest.join(name);
            self.log.push(AppError::NoPassword { path }.to_string());
            return;
        }
        let output = ArchiveOutput {
            format,
            name: OsString::from(name),
            password: (!password.is_empty()).then(|| password.to_owned()),
        };
        let sources = source_paths(&dir, &names);
        self.begin_operation(
            OperationKind::Compress,
            dir,
            names,
            sources,
            Some(dest),
            Some(output),
            None,
            effects,
        );
    }

    pub(super) fn confirm_delete(&mut self) {
        if let Some((location, names)) = self.target_names(OperationKind::Delete.verb()) {
            let popup = ChoicePopup::delete_confirm(location.base, names);
            self.popup = Some(Popup::Choice(popup));
        }
    }

    // The directory and names come from the confirmation when there was one, so
    // that what was asked about is what goes.
    pub(super) fn start_removal(
        &mut self,
        kind: OperationKind,
        dir: PathBuf,
        names: Vec<OsString>,
        effects: &mut Vec<Effect>,
    ) {
        let sources = source_paths(&dir, &names);
        self.start_operation(kind, dir, names, sources, None, effects);
    }

    fn start_calculation(&mut self, kind: OperationKind, effects: &mut Vec<Effect>) {
        let Some((location, names)) = self.target_names(kind.verb()) else {
            return;
        };
        self.start_removal(kind, location.base, names, effects);
    }

    pub(super) fn start_find(&mut self, dir: PathBuf, pattern: Filter, effects: &mut Vec<Effect>) {
        let names = vec![dir.clone().into_os_string()];
        let sources = Sources::Find(FindQuery {
            dir: dir.clone(),
            pattern,
            show_hidden: self.active_pane().show_hidden(),
            max_matches: self.config.filelist.max_find_matches,
        });
        self.start_operation(OperationKind::Find, dir, names, sources, None, effects);
    }

    pub(super) fn confirm_quit(&mut self) {
        if let Some(Popup::Choice(ChoicePopup {
            kind: ChoiceKind::QuitDuringOperation { .. },
            ..
        })) = self.popup
        {
            return;
        }
        let cancelling = self
            .operation
            .as_ref()
            .is_some_and(|operation| operation.quit_after);
        let resume = self.popup.take();
        self.popup = Some(Popup::Choice(ChoicePopup::quit_during_operation(
            resume, cancelling,
        )));
    }

    pub(super) fn continue_operation(&mut self, resume: Option<Box<Popup>>) {
        self.popup = resume.map(|popup| *popup);
    }

    pub(super) fn cancel_and_quit(&mut self, effects: &mut Vec<Effect>) {
        let Some(operation) = &mut self.operation else {
            return;
        };
        // A worker blocked in a system call, as on a mount that stopped
        // responding, never reports that it finished, so a second confirmation
        // quits without waiting for it.
        if operation.quit_after {
            self.quit(true, effects);
            return;
        }
        operation.quit_after = true;
        self.cancel_operation(effects);
    }

    // Keys while an operation runs: a popup of the operation takes them, and
    // otherwise only Esc and the key for a new window do anything.
    pub(super) fn operation_key(&mut self, key: Key, effects: &mut Vec<Effect>) {
        let is_new_window =
            self.config.keys.lookup(Context::FileList, &key) == Some(Command::NewWindow);
        if is_new_window && !matches!(self.popup, Some(Popup::Input(_))) {
            self.new_window(false, effects);
            return;
        }
        if self.popup.is_some() {
            self.popup_key(key, effects);
        } else if key.base == KeyBase::Named(NamedKey::Escape) && !key.modified() {
            self.cancel_operation(effects);
        }
    }

    pub(super) fn cancel_operation(&mut self, effects: &mut Vec<Effect>) {
        if let Some(operation) = &mut self.operation
            && !operation.cancelling
        {
            operation.cancelling = true;
            effects.push(Effect::OperationReply(Reply::Cancel));
        }
    }

    pub(super) fn reply(&mut self, reply: Reply, effects: &mut Vec<Effect>) {
        if reply == Reply::Cancel {
            self.cancel_operation(effects);
        } else {
            effects.push(Effect::OperationReply(reply));
        }
    }

    pub(super) fn confirm_password(&mut self, popup: InputPopup, effects: &mut Vec<Effect>) {
        let password = popup.text(0).to_owned();
        if password.is_empty() {
            self.popup = Some(Popup::Input(popup));
            return;
        }
        if let InputKind::ArchivePassword { level, .. } = popup.kind
            && let Some(operation) = &self.operation
            && level < operation.remembered_levels
        {
            self.panes[operation.side.index()].set_archive_password(level, Some(password.clone()));
        }
        self.reply(Reply::Password(password), effects);
    }

    pub(super) fn operation_event(&mut self, event: OperationEvent, effects: &mut Vec<Effect>) {
        let Some(operation) = &mut self.operation else {
            return;
        };
        match event {
            OperationEvent::Progress { current, done } => {
                operation.current = Some(current);
                operation.done = done;
            }
            OperationEvent::ItemDone { index } => {
                // The names of a paste are those of what it copies, not of
                // items in the pane.
                if operation.kind != OperationKind::ExtractToTemp
                    && !operation.paste
                    && let Some(name) = operation.names.get(index)
                {
                    self.panes[operation.side.index()].unmark(name);
                }
                if let Some(done) = operation
                    .renames
                    .as_mut()
                    .and_then(|renames| renames.done.get_mut(index))
                {
                    *done = true;
                }
            }
            OperationEvent::Skipped { path, error } => {
                if let Some(temp) = &mut operation.temp {
                    temp.entry_skipped = true;
                }
                self.log
                    .push(format!("Skipped {}: {error}", path.display()));
            }
            OperationEvent::Conflict { source, dest } => {
                self.show_operation_popup(Popup::Conflict(ConflictPopup::new(source, dest, false)));
            }
            OperationEvent::NeedPassword {
                archive,
                level,
                retry,
            } => {
                if retry && level < operation.remembered_levels {
                    self.panes[operation.side.index()].set_archive_password(level, None);
                }
                self.show_operation_popup(Popup::Input(InputPopup::password(
                    archive,
                    level,
                    retry,
                    PasswordPurpose::Operation,
                )));
            }
            OperationEvent::TempDir(path) => operation.temp_dir = Some(path),
            // The marks are kept by name, and the names in the cycle have
            // moved round, so a mark left there could be on another item.
            OperationEvent::CycleBroken { cycle, left, temp } => {
                for name in cycle.iter().filter_map(|&index| operation.names.get(index)) {
                    self.panes[operation.side.index()].unmark(name);
                }
                if let Some(renames) = &mut operation.renames
                    && let Some((old, _)) = renames.pairs.get(left)
                {
                    renames.left = Some((old.clone(), temp));
                }
            }
            OperationEvent::Finished(outcome) => self.finished(&outcome, effects),
        }
    }

    pub(super) fn operation_view(&self) -> Option<PopupView> {
        let operation = self.operation.as_ref()?;
        let title = if operation.cancelling {
            "Cancelling"
        } else {
            operation.kind.title()
        };
        let current = operation
            .current
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        Some(PopupView {
            title: title.to_owned(),
            message: Some(format!(
                "{} {}: {current}",
                operation.done,
                operation.kind.progress()
            )),
            progress: true,
            ..PopupView::default()
        })
    }

    pub(super) fn open_rename(&mut self) {
        let Some((location, entry)) = self.active_pane().cursor_target() else {
            return;
        };
        if let Some(error) = self.bad_target("rename", &location, [entry.name.as_os_str()]) {
            self.log.push(error.to_string());
            return;
        }
        let text = entry.name.to_string_lossy().into_owned();
        let selection = rename_selection(&entry);
        let kind = InputKind::Rename {
            dir: location.base,
            original: entry.name,
        };
        self.popup = Some(Popup::Input(InputPopup::single(kind, text, selection)));
    }

    fn open_batch_rename(&mut self, mode: RenameMode) {
        let pane = self.active_pane();
        let Some(targets) = pane.targets() else {
            return;
        };
        let names = targets.entries.iter().map(|entry| entry.name.as_os_str());
        if let Some(error) = self.bad_target("rename", &targets.location, names) {
            self.log.push(error.to_string());
            return;
        }
        let siblings = targets.location == *pane.location();
        let dir = targets.location.base;
        // A name that is not UTF-8 would not survive being put together again
        // as text.
        if let Some(target) = targets
            .entries
            .iter()
            .find(|entry| entry.name.to_str().is_none())
        {
            let path = dir.join(&target.name);
            self.log.push(AppError::NotUtf8 { path }.to_string());
            return;
        }
        let popup = RenamePopup::new(
            mode,
            dir,
            targets.entries,
            siblings,
            pane.listing().entries(),
            self.os,
        );
        self.popup = Some(Popup::Rename(popup));
    }

    pub(super) fn confirm_batch_rename(&mut self, popup: RenamePopup, effects: &mut Vec<Effect>) {
        let pairs = popup.pairs();
        if popup.plan.error.is_some() || pairs.is_empty() {
            self.popup = Some(Popup::Rename(popup));
            return;
        }
        let pane = &mut self.panes[self.active.index()];
        for (target, new) in popup.targets.iter().zip(&popup.plan.names) {
            if new.is_none() {
                pane.unmark(&target.name);
            }
        }
        let cursor = pane
            .cursor_entry()
            .filter(|_| pane.location().base == popup.dir)
            .and_then(|entry| pairs.iter().position(|(old, _)| entry.name == **old));
        let names = pairs.iter().map(|(old, _)| OsString::from(old)).collect();
        let sources = Sources::Renames(Renames {
            dir: popup.dir.clone(),
            pairs: pairs.clone(),
        });
        self.start_operation(
            OperationKind::Rename,
            popup.dir,
            names,
            sources,
            None,
            effects,
        );
        if let Some(operation) = &mut self.operation {
            operation.renames = Some(RenameProgress {
                done: vec![false; pairs.len()],
                pairs,
                cursor,
                left: None,
            });
        }
    }

    pub(super) fn confirm_name(&mut self, kind: InputKind, text: &str, effects: &mut Vec<Effect>) {
        if text.is_empty() {
            return;
        }
        let base = match &kind {
            InputKind::Rename { dir, .. }
            | InputKind::NewDir { dir }
            | InputKind::NewFile { dir }
            | InputKind::NewText { dir } => dir.clone(),
            InputKind::ConflictRename { .. }
            | InputKind::Filter
            | InputKind::MarkGlob
            | InputKind::GoToPath { .. }
            | InputKind::Find { .. }
            | InputKind::Run
            | InputKind::RunCommandLine
            | InputKind::ArchivePassword { .. }
            | InputKind::Compress { .. } => return,
        };
        if base.as_os_str().is_empty() {
            return;
        }
        match kind {
            InputKind::Rename { original, .. } => {
                // A name that only differs in normalization is the same entry
                // on some file systems, so it is never renamed to one.
                if same_name(&original.to_string_lossy(), text) {
                    return;
                }
                if let Some(problem) = name_problem(text, self.os) {
                    let error = AppError::BadNewName {
                        path: base.join(&original),
                        name: text.to_owned(),
                        reason: problem.reason(),
                    };
                    self.log.push(error.to_string());
                } else {
                    effects.push(Effect::Rename {
                        from: base.join(&original),
                        to: base.join(text),
                    });
                }
            }
            InputKind::NewDir { .. } => {
                let checked = dir_below(&base, text, self.os);
                if let Some(path) = self.created_path(&base, text, checked) {
                    effects.push(Effect::Mkdir(path));
                }
            }
            InputKind::NewFile { .. } => {
                let checked = file_below(&base, text, self.os);
                if let Some(path) = self.created_path(&base, text, checked) {
                    effects.push(Effect::CreateFile(path));
                }
            }
            InputKind::NewText { .. } => {
                let checked = file_below(&base, text, self.os);
                let Some(path) = self.created_path(&base, text, checked) else {
                    return;
                };
                // Some editors do nothing when given a path that does not
                // exist yet (`open -t` on macOS), so the file is made first by
                // default.
                if self.config.filelist.new_text_creates_file {
                    self.edit_after_create = Some((path.clone(), base));
                    effects.push(Effect::CreateFile(path));
                } else {
                    let expansion = Expansion {
                        dir: base,
                        ..self.file_expansion(path)
                    };
                    self.spawn_editor(&expansion, effects);
                }
            }
            InputKind::ConflictRename { .. }
            | InputKind::Filter
            | InputKind::MarkGlob
            | InputKind::GoToPath { .. }
            | InputKind::Find { .. }
            | InputKind::Run
            | InputKind::RunCommandLine
            | InputKind::ArchivePassword { .. }
            | InputKind::Compress { .. } => {}
        }
    }

    pub(super) fn changed(
        &mut self,
        change: Change,
        error: Option<IoError>,
        effects: &mut Vec<Effect>,
    ) {
        match (change, error) {
            (Change::Renamed { from, to }, None) => {
                self.log.push(format!(
                    "Renamed {} to {}",
                    from.file_name().unwrap_or_default().to_string_lossy(),
                    to.file_name().unwrap_or_default().to_string_lossy()
                ));
                self.follow_renamed(&[(from.clone(), to.clone())], effects);
                if let Some(dir) = to.parent() {
                    self.show_created(dir, to.file_name().map(OsString::from), effects);
                }
                if let Some(dir) = from.parent()
                    && Some(dir) != to.parent()
                {
                    self.refresh_showing(dir, effects);
                }
            }
            (Change::Renamed { from, .. }, Some(source)) => {
                self.log
                    .push(AppError::RenameFailed { path: from, source }.to_string());
            }
            (Change::DirCreated(path), None) => {
                self.log.push(self.created_line(&path));
                if self.config.filelist.new_dir_goes_to_dir {
                    self.navigate(
                        self.active,
                        Location::new(path),
                        None,
                        HistoryMove::Visit,
                        effects,
                    );
                } else {
                    let base = self.active_pane().location().base.clone();
                    let first = first_component_under(&path, &base);
                    self.show_created(&base, first, effects);
                }
            }
            (Change::FileCreated(path), None) => {
                self.log.push(self.created_line(&path));
                if let Some((edit, dir)) = self.edit_after_create.take()
                    && edit == path
                {
                    let expansion = Expansion {
                        dir,
                        ..self.file_expansion(path.clone())
                    };
                    self.spawn_editor(&expansion, effects);
                }
                if let Some(dir) = path.parent() {
                    self.show_created(dir, path.file_name().map(OsString::from), effects);
                }
            }
            (Change::DirCreated(path) | Change::FileCreated(path), Some(source)) => {
                self.edit_after_create = None;
                self.log
                    .push(AppError::CreateFailed { path, source }.to_string());
            }
        }
    }

    pub(super) fn edit(&mut self, effects: &mut Vec<Effect>) {
        let Some((location, entry)) = self.active_pane().cursor_target() else {
            return;
        };
        if entry.kind.is_dir_like() {
            return;
        }
        if let Some(error) = self.bad_target("edit", &location, [entry.name.as_os_str()]) {
            self.log.push(error.to_string());
            return;
        }
        if entry.kind.is_special() {
            let path = location.base.join(&entry.name);
            self.log.push(AppError::NotRegularFile { path }.to_string());
            return;
        }
        if let Some(error) = link_in_archive("edit", &location, [&entry]) {
            self.log.push(error.to_string());
            return;
        }
        if location.in_archive() {
            let temp = Temp {
                action: TempAction::Edit,
                from: location.clone(),
                targets: vec![entry.name.clone()],
                entry_skipped: false,
            };
            let entries = self.archive_entries(&location, std::slice::from_ref(&entry.name));
            self.start_temp(temp, entries, effects);
        } else {
            self.edit_file(location.base.join(&entry.name), effects);
        }
    }

    pub(super) fn edit_source(&mut self, source: PreviewSource, effects: &mut Vec<Effect>) {
        let from = source.location();
        if let Some((path, name)) = source.entry {
            let temp = Temp {
                action: TempAction::Edit,
                from,
                targets: vec![OsString::from(&name)],
                entry_skipped: false,
            };
            let entries = ArchiveEntries {
                archive: source.path,
                path,
                names: vec![name],
                passwords: source.passwords,
            };
            self.start_temp(temp, entries, effects);
        } else {
            self.edit_file(source.path, effects);
        }
    }

    pub(super) fn open_with_os(&mut self, effects: &mut Vec<Effect>) {
        let Some((location, entry)) = self.active_pane().cursor_target() else {
            return;
        };
        if let Some(error) = self.bad_target("open", &location, [entry.name.as_os_str()]) {
            self.log.push(error.to_string());
            return;
        }
        if let Some(error) = link_in_archive("open", &location, [&entry]) {
            self.log.push(error.to_string());
            return;
        }
        if location.in_archive() {
            let entries = self.archive_entries(&location, std::slice::from_ref(&entry.name));
            let temp = Self::temp_open(&location, entry.name);
            self.start_temp(temp, entries, effects);
        } else {
            effects.push(Effect::OpenWithOs(location.base.join(&entry.name)));
        }
    }

    pub(super) fn run(&mut self, effects: &mut Vec<Effect>) {
        let Some((location, entry)) = self.active_pane().cursor_target() else {
            return;
        };
        if let Some(error) = self.bad_target("run", &location, [entry.name.as_os_str()]) {
            self.log.push(error.to_string());
            return;
        }
        if let Some(error) = link_in_archive("run", &location, [&entry]) {
            self.log.push(error.to_string());
            return;
        }
        let executable = entry.kind.is_executable();
        if location.in_archive() {
            let temp = Temp {
                action: TempAction::Run { executable },
                from: location.clone(),
                targets: vec![entry.name.clone()],
                entry_skipped: false,
            };
            let entries = self.archive_entries(&location, std::slice::from_ref(&entry.name));
            self.start_temp(temp, entries, effects);
        } else {
            self.open_run(&location.base.join(&entry.name), executable);
        }
    }

    // A line uses one kind of path placeholder, so what it runs on is the
    // entries of one place: inside an archive, one extraction.
    pub(super) fn start_line(
        &mut self,
        name: SpawnName,
        line: CommandLine,
        effects: &mut Vec<Effect>,
    ) {
        let pane = self.active_pane();
        let targets = match line.paths_placeholder() {
            None | Some(Placeholder::Dir | Placeholder::OtherDir) => None,
            Some(Placeholder::MarkedPaths) => Some(Targets {
                location: pane.location().clone(),
                entries: pane.marked_entries().cloned().collect(),
            }),
            Some(Placeholder::Paths) => pane.targets(),
            Some(Placeholder::CursorPath | Placeholder::Path) => {
                pane.cursor_target().map(|(location, entry)| Targets {
                    location,
                    entries: vec![entry],
                })
            }
        };
        let expansion = self.expansion();
        let Some(targets) = targets else {
            self.spawn(name, &line, &expansion, effects);
            return;
        };
        let names: Vec<OsString> = targets
            .entries
            .iter()
            .map(|entry| entry.name.clone())
            .collect();
        let error = self
            .bad_target(
                "run a command on",
                &targets.location,
                names.iter().map(OsString::as_os_str),
            )
            .or_else(|| link_in_archive("run a command on", &targets.location, &targets.entries));
        if let Some(error) = error {
            self.log.push(error.to_string());
            return;
        }
        // The `..` row at the root of an archive on disk is the archive file
        // itself, which needs no extraction. A line that is not going to
        // start has nothing extracted for it.
        if !targets.location.in_archive() || line.expand(&expansion).is_err() {
            self.spawn(name, &line, &expansion, effects);
            return;
        }
        let entries = self.archive_entries(&targets.location, &names);
        let temp = Temp {
            action: TempAction::Spawn { name, line },
            from: targets.location,
            targets: names,
            entry_skipped: false,
        };
        self.start_temp(temp, entries, effects);
    }

    pub(super) fn start_on_entry(
        &mut self,
        name: SpawnName,
        line: CommandLine,
        location: &Location,
        entry: &Entry,
        effects: &mut Vec<Effect>,
    ) {
        if line.uses_paths()
            && let Some(error) = self.bad_target("open", location, [entry.name.as_os_str()])
        {
            self.log.push(error.to_string());
            return;
        }
        if location.in_archive() && line.uses_paths() {
            let temp = Temp {
                action: TempAction::Spawn { name, line },
                from: location.clone(),
                targets: vec![entry.name.clone()],
                entry_skipped: false,
            };
            let entries = self.archive_entries(location, std::slice::from_ref(&entry.name));
            self.start_temp(temp, entries, effects);
        } else {
            let expansion = self.file_expansion(location.base.join(&entry.name));
            self.spawn(name, &line, &expansion, effects);
        }
    }

    pub(super) fn copy_to_clipboard(&mut self, full_paths: bool, effects: &mut Vec<Effect>) {
        let Some(targets) = self.active_pane().targets() else {
            return;
        };
        let lines: Vec<String> = targets
            .entries
            .iter()
            .map(|entry| {
                if full_paths {
                    targets.location.child_display(&entry.name)
                } else {
                    entry.name.to_string_lossy().into_owned()
                }
            })
            .collect();
        effects.push(Effect::Clipboard(lines.join("\n")));
    }

    pub(super) fn copy_files(&mut self, effects: &mut Vec<Effect>) {
        let Some(targets) = self.active_pane().targets() else {
            return;
        };
        let names = targets.entries.iter().map(|entry| entry.name.as_os_str());
        if let Some(error) = self.bad_target("copy", &targets.location, names) {
            self.log.push(error.to_string());
            return;
        }
        // arboard resolves every path before putting it on the clipboard, so
        // a link would be pasted as its target, under the target's name, and
        // a broken one would be silently left out.
        if let Some(link) = targets
            .entries
            .iter()
            .find(|entry| matches!(entry.kind, EntryKind::Symlink { .. }))
        {
            let path = targets.location.base.join(&link.name);
            self.log
                .push(AppError::LinkToClipboard { path }.to_string());
            return;
        }
        let paths = targets
            .entries
            .iter()
            .map(|entry| targets.location.base.join(&entry.name))
            .collect();
        effects.push(Effect::ClipboardFiles(paths));
    }

    pub(super) fn drag_files(&mut self, effects: &mut Vec<Effect>) {
        let Some(targets) = self.active_pane().targets() else {
            return;
        };
        // Nothing is extracted to drag it out: the drag has to start while
        // the button is held, which leaves no time to wait for an extraction
        // or to ask for a password.
        if targets.location.in_archive() {
            self.log.push(AppError::DragInArchive.to_string());
            return;
        }
        let names = targets.entries.iter().map(|entry| entry.name.as_os_str());
        if let Some(error) = self.bad_target("drag", &targets.location, names) {
            self.log.push(error.to_string());
            return;
        }
        // Refused as by `copy_files`, so that the two ways of handing files
        // to another application take the same ones.
        if let Some(link) = targets
            .entries
            .iter()
            .find(|entry| matches!(entry.kind, EntryKind::Symlink { .. }))
        {
            let path = targets.location.base.join(&link.name);
            self.log.push(AppError::LinkDragged { path }.to_string());
            return;
        }
        let paths = targets
            .entries
            .iter()
            .map(|entry| targets.location.base.join(&entry.name))
            .collect();
        effects.push(Effect::DragFiles(paths));
    }

    // A paste always copies, even what another application cut: arboard can
    // neither read nor write the mark that says files were cut, so a cut
    // would need code of wafu's own for each clipboard, and macOS has no such
    // mark at all. Moving is what `move` is for.
    pub(super) fn paste_files(&mut self, effects: &mut Vec<Effect>) {
        let location = self.active_pane().location();
        if location.in_archive() {
            self.log.push(AppError::PasteIntoArchive.to_string());
            return;
        }
        if location.base.as_os_str().is_empty() {
            self.log.push(AppError::NoPasteLocation.to_string());
            return;
        }
        let dest = location.base.clone();
        effects.push(Effect::ReadClipboardFiles { dest });
    }

    pub(super) fn clipboard_files_read(
        &mut self,
        dest: PathBuf,
        result: Result<Vec<PathBuf>, IoError>,
        effects: &mut Vec<Effect>,
    ) {
        let paths = match result {
            Ok(paths) if paths.is_empty() => {
                self.log.push(AppError::NoFilesToPaste.to_string());
                return;
            }
            Ok(paths) => paths,
            Err(source) => {
                self.log.push(AppError::PasteFailed { source }.to_string());
                return;
            }
        };
        // The answer comes back through the event loop, so a key pressed in
        // between may have started an operation or opened a popup.
        if self.operation.is_some() || self.popup.is_some() {
            self.log.push(AppError::PasteInterrupted.to_string());
            return;
        }
        let names = paths
            .iter()
            .map(|path| path.file_name().unwrap_or_default().to_owned())
            .collect();
        self.start_operation(
            OperationKind::Copy,
            dest.clone(),
            names,
            Sources::Paths(paths),
            Some(dest),
            effects,
        );
        if let Some(operation) = &mut self.operation {
            operation.paste = true;
        }
    }

    // The passwords the pane knows for the archive levels `location` shares
    // with the pane's.
    pub(super) fn passwords_for(&self, location: &Location) -> Passwords {
        let pane = self.active_pane();
        pane.archive_passwords()
            .truncated(location.shared_levels(pane.location()))
    }

    // The location a command works in: that of what it acts on, or the pane's
    // for one that makes something new there. Commands that open or `rename` look
    // at the cursor alone, since following the marks would launch one
    // application per marked file; the rest take the marks, else the cursor.
    // Mistakes there are guarded by `require_marks` and the delete confirmation.
    fn command_location(&self, command: &Command) -> Location {
        let pane = self.active_pane();
        let location = match command {
            // On the `..` row of an archive root the target of `extract_here`
            // and `compress_here` is outside the archive, but the pane does
            // not show where the result would land.
            Command::NewDir
            | Command::NewFile
            | Command::NewText
            | Command::ExtractHere
            | Command::CompressHere => None,
            Command::OpenWithOs | Command::Rename | Command::Edit | Command::OpenAsArchive => {
                pane.cursor_target().map(|(location, _)| location)
            }
            _ => pane.targets().map(|targets| targets.location),
        };
        location.unwrap_or_else(|| pane.location().clone())
    }

    // Special files are not weeded out here: the worker refuses them, and it
    // alone sees the ones inside directories.
    fn target_names(&mut self, verb: &'static str) -> Option<(Location, Vec<OsString>)> {
        let targets = self.active_pane().targets()?;
        let names = targets.entries.iter().map(|entry| entry.name.as_os_str());
        if let Some(error) = self.bad_target(verb, &targets.location, names) {
            self.log.push(error.to_string());
            return None;
        }
        let names = targets
            .entries
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        Some((targets.location, names))
    }

    // Windows drops a trailing dot or space from a path element, so a path
    // made from such a name would name a sibling.
    pub(super) fn bad_target<'a>(
        &self,
        verb: &'static str,
        location: &Location,
        names: impl IntoIterator<Item = &'a OsStr>,
    ) -> Option<AppError> {
        if self.os != Os::Windows || location.in_archive() {
            return None;
        }
        let name = names.into_iter().find(|name| {
            matches!(
                name_problem(&name.to_string_lossy(), Os::Windows),
                Some(NameProblem::Ending)
            )
        })?;
        Some(AppError::BadTargetName {
            verb,
            path: location.base.join(name),
            reason: NameProblem::Ending.reason(),
        })
    }

    fn edit_file(&mut self, path: PathBuf, effects: &mut Vec<Effect>) {
        let expansion = self.file_expansion(path);
        self.spawn_editor(&expansion, effects);
    }

    // A pane showing what was renamed follows it to its new name. One
    // showing a place inside it is not followed, as after a rename made
    // outside wafu.
    fn follow_renamed(&mut self, renamed: &[(PathBuf, PathBuf)], effects: &mut Vec<Effect>) {
        for side in [Side::Left, Side::Right] {
            let pane = &self.panes[side.index()];
            let Some((_, to)) = renamed
                .iter()
                .find(|(from, _)| pane.location().base == *from)
            else {
                continue;
            };
            let target = Location {
                base: to.clone(),
                archive: pane.location().archive.clone(),
            };
            let cursor = pane.cursor_entry().map(|entry| entry.name.clone());
            self.navigate(side, target, cursor, HistoryMove::Visit, effects);
        }
    }

    // The other pane's directory as a destination, or `None` with the reason logged.
    fn other_dest(&mut self, verb: &'static str) -> Option<PathBuf> {
        let other = self.pane(self.active.other()).location();
        if other.base.as_os_str().is_empty() {
            self.log
                .push(AppError::NoOtherLocation { verb }.to_string());
            return None;
        }
        if other.in_archive() {
            self.log.push(AppError::IntoArchive { verb }.to_string());
            return None;
        }
        Some(other.base.clone())
    }

    fn temp_open(location: &Location, name: OsString) -> Temp {
        Temp {
            action: TempAction::OpenWithOs,
            from: location.clone(),
            targets: vec![name],
            entry_skipped: false,
        }
    }

    fn archive_entries(&self, location: &Location, names: &[OsString]) -> ArchiveEntries {
        let path = location
            .archive
            .clone()
            .expect("archive entries are only made inside an archive");
        ArchiveEntries {
            archive: location.base.clone(),
            path,
            names: names
                .iter()
                .map(|name| name.to_string_lossy().into_owned())
                .collect(),
            passwords: self.passwords_for(location),
        }
    }

    fn start_temp(&mut self, temp: Temp, entries: ArchiveEntries, effects: &mut Vec<Effect>) {
        let names = entries.names.iter().map(OsString::from).collect();
        let source_dir = entries.archive.clone();
        self.begin_operation(
            OperationKind::ExtractToTemp,
            source_dir,
            names,
            Sources::Entries(entries),
            None,
            None,
            Some(temp),
            effects,
        );
    }

    fn start_operation(
        &mut self,
        kind: OperationKind,
        source_dir: PathBuf,
        names: Vec<OsString>,
        sources: Sources,
        dest: Option<PathBuf>,
        effects: &mut Vec<Effect>,
    ) {
        self.begin_operation(kind, source_dir, names, sources, dest, None, None, effects);
    }

    #[allow(clippy::too_many_arguments)]
    fn begin_operation(
        &mut self,
        kind: OperationKind,
        source_dir: PathBuf,
        names: Vec<OsString>,
        sources: Sources,
        dest: Option<PathBuf>,
        output: Option<ArchiveOutput>,
        temp: Option<Temp>,
        effects: &mut Vec<Effect>,
    ) {
        if names.is_empty() {
            return;
        }
        if source_dir.as_os_str().is_empty() {
            return;
        }
        let remembered_levels = match &sources {
            Sources::Entries(entries) => Location {
                base: entries.archive.clone(),
                archive: Some(entries.path.clone()),
            }
            .shared_levels(self.active_pane().location()),
            Sources::Paths(_) | Sources::Find(_) | Sources::Renames(_) => 0,
        };
        effects.push(Effect::StartOperation(OperationSpec {
            kind,
            sources,
            dest: dest.clone(),
            output,
        }));
        self.operation = Some(Operation {
            kind,
            side: self.active,
            names,
            source_dir,
            dest,
            current: None,
            done: 0,
            cancelling: false,
            quit_after: false,
            temp,
            temp_dir: None,
            remembered_levels,
            renames: None,
            paste: false,
        });
    }

    // A question that arrives behind the quit confirmation waits there and
    // comes up when the operation is continued.
    fn show_operation_popup(&mut self, popup: Popup) {
        if let Some(Popup::Choice(ChoicePopup {
            kind: ChoiceKind::QuitDuringOperation { resume },
            ..
        })) = &mut self.popup
        {
            *resume = Some(Box::new(popup));
        } else {
            self.popup = Some(popup);
        }
    }

    fn finished(&mut self, outcome: &Outcome, effects: &mut Vec<Effect>) {
        let Some(operation) = self.operation.take() else {
            return;
        };
        self.popup = None;
        // Another program is not handed a set of files with some left out.
        // Conflicts answered with Skip count as skipped too, but the user
        // chose them.
        let launch = matches!(outcome, Outcome::Completed { .. })
            && operation
                .temp
                .as_ref()
                .is_some_and(|temp| !temp.entry_skipped);
        let line = || {
            if let Some(line) = operation.temp.as_ref().and_then(|temp| temp.line(outcome)) {
                return line;
            }
            let line = OutcomeLine {
                kind: operation.kind,
                paste: operation.paste,
                outcome,
                targets: &operation.names,
            }
            .to_string();
            match &operation.renames {
                Some(renames) => renames.line(outcome, line),
                None => line,
            }
        };
        if let Outcome::Found { paths, truncated } = outcome {
            if *truncated {
                self.log.push(line());
            }
            self.popup = Some(Popup::List(ListPopup::found(
                &operation.source_dir,
                paths.clone(),
            )));
        } else if !launch {
            self.log.push(line());
        }
        if let Some(renames) = &operation.renames {
            self.follow_batch_rename(operation.side, &operation.source_dir, renames, effects);
        }
        // A size calculation or a find changes nothing to read again.
        if !matches!(
            operation.kind,
            OperationKind::CalculateSize | OperationKind::CalculateSizeOnDisk | OperationKind::Find
        ) {
            self.refresh_showing(&operation.source_dir, effects);
        }
        if let Some(dest) = &operation.dest
            && *dest != operation.source_dir
        {
            self.refresh_showing(dest, effects);
        }
        if launch
            && let Some(temp) = operation.temp
            && let Some(dir) = operation.temp_dir
        {
            self.run_temp_action(temp, &dir, effects);
        }
        if operation.quit_after {
            self.quit(true, effects);
        }
    }

    fn run_temp_action(&mut self, temp: Temp, dir: &Path, effects: &mut Vec<Effect>) {
        let paths: Vec<PathBuf> = temp.targets.iter().map(|name| dir.join(name)).collect();
        // A line uses one kind of path placeholder, and the targets are the
        // entries that kind stands for.
        let expansion = Expansion {
            path: paths.first().cloned(),
            cursor_path: paths.first().cloned(),
            marked_paths: paths.clone(),
            paths,
            dir: self.active_pane().location().dir().to_path_buf(),
            other_dir: self
                .pane(self.active.other())
                .location()
                .dir()
                .to_path_buf(),
        };
        match temp.action {
            TempAction::Edit => {
                self.spawn_editor(&expansion, effects);
            }
            TempAction::OpenWithOs => {
                if let Some(path) = expansion.paths.first() {
                    effects.push(Effect::OpenWithOs(path.clone()));
                }
            }
            TempAction::Run { executable } => {
                if let Some(path) = expansion.paths.first() {
                    self.open_run(path, executable);
                }
            }
            TempAction::Spawn { name, line } => self.spawn(name, &line, &expansion, effects),
        }
    }

    // After a batch rename, whether it ran to the end or not: the cursor
    // stays on the item it was on, and a pane showing a renamed directory
    // follows it, as after `rename`.
    fn follow_batch_rename(
        &mut self,
        side: Side,
        dir: &Path,
        progress: &RenameProgress,
        effects: &mut Vec<Effect>,
    ) {
        let renamed: Vec<&(String, String)> = progress
            .pairs
            .iter()
            .zip(&progress.done)
            .filter_map(|(pair, &done)| done.then_some(pair))
            .collect();
        let pane = &mut self.panes[side.index()];
        if pane.location().base == dir
            && let Some(cursor) = progress.cursor
            && progress.done[cursor]
        {
            let (_, new) = &progress.pairs[cursor];
            pane.set_pending_cursor(Some(OsString::from(new)));
        }
        let paths: Vec<(PathBuf, PathBuf)> = renamed
            .iter()
            .map(|(old, new)| (dir.join(old), dir.join(new)))
            .collect();
        self.follow_renamed(&paths, effects);
    }

    fn created_path(
        &mut self,
        base: &Path,
        text: &str,
        checked: Result<PathBuf, NameProblem>,
    ) -> Option<PathBuf> {
        checked
            .map_err(|problem| {
                let error = AppError::BadCreateName {
                    path: base.join(text),
                    reason: problem.reason(),
                };
                self.log.push(error.to_string());
            })
            .ok()
    }

    fn created_line(&self, path: &Path) -> String {
        let relative = [self.active, self.active.other()]
            .into_iter()
            .find_map(|side| {
                let rest = path.strip_prefix(&self.pane(side).location().base).ok()?;
                (!rest.as_os_str().is_empty()).then(|| rest.components().collect::<PathBuf>())
            });
        let shown = relative.unwrap_or_else(|| path.to_path_buf());
        format!("Created {}", shown.display())
    }

    // Puts the cursor on something just made or renamed in the active pane and
    // reads every pane showing its directory again.
    fn show_created(&mut self, dir: &Path, name: Option<OsString>, effects: &mut Vec<Effect>) {
        if self.active_pane().location().base == dir {
            self.active_pane_mut().set_pending_cursor(name);
        }
        self.refresh_showing(dir, effects);
    }

    fn refresh_showing(&mut self, path: &Path, effects: &mut Vec<Effect>) {
        for side in [Side::Left, Side::Right] {
            if self.panes[side.index()].location().base == path {
                self.refresh(side, effects);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Operation {
    kind: OperationKind,
    side: Side,
    names: Vec<OsString>,
    // The directory of the sources, or the archive they are in.
    source_dir: PathBuf,
    dest: Option<PathBuf>,
    current: Option<PathBuf>,
    done: usize,
    cancelling: bool,
    quit_after: bool,
    temp: Option<Temp>,
    temp_dir: Option<PathBuf>,
    // A password typed for one of the first this many archive levels of the
    // sources is kept by the pane, whose location has them too. An archive
    // unpacked by an extract is past them.
    remembered_levels: usize,
    renames: Option<RenameProgress>,
    // A copy from the clipboard into the pane. Its sources may be anywhere,
    // and only `dest` changes, so `source_dir` is `dest`.
    paste: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Temp {
    action: TempAction,
    from: Location,
    // The entries the path placeholder of the line stands for.
    targets: Vec<OsString>,
    entry_skipped: bool,
}

impl Temp {
    // The extraction is a step of the command the user ran, so a line that
    // tells how it ended names that command and what it was run on.
    fn line(&self, outcome: &Outcome) -> Option<String> {
        let (verb, noun) = self.action.words();
        let subject = match self.targets.as_slice() {
            [name] => self.from.child_display(name),
            targets => format!("{} items", targets.len()),
        };
        match outcome {
            Outcome::Completed { skipped } => Some(format!(
                "Cannot {verb} {subject}: {skipped} {} skipped",
                plural(*skipped, "entry was", "entries were")
            )),
            Outcome::Cancelled => Some(format!("{noun} cancelled")),
            Outcome::Failed { error, .. } => Some(format!("Cannot {verb} {subject}: {error}")),
            Outcome::Found { .. } | Outcome::Measured { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TempAction {
    Edit,
    OpenWithOs,
    Run { executable: bool },
    Spawn { name: SpawnName, line: CommandLine },
}

impl TempAction {
    fn words(&self) -> (&'static str, &'static str) {
        match self {
            Self::Edit => ("edit", "Edit"),
            Self::OpenWithOs
            | Self::Spawn {
                name: SpawnName::Command(Command::OpenAlternative),
                ..
            } => ("open", "Open"),
            Self::Run { .. } => ("run", "Run"),
            Self::Spawn { .. } => ("run a command on", "Command"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RenameProgress {
    pairs: Vec<(String, String)>,
    done: Vec<bool>,
    // The pair of the item the cursor was on at Enter. Reading the cursor
    // when the operation ends could find another item: reloads during it
    // move the cursor by name, and an old name can be taken by another item.
    cursor: Option<usize>,
    // The old name of the item a failure left under a temporary name, and that name.
    left: Option<(String, String)>,
}

impl RenameProgress {
    fn line(&self, outcome: &Outcome, line: String) -> String {
        match (outcome, self.pairs.as_slice(), &self.left) {
            (Outcome::Completed { .. }, [(old, new)], _) => format!("Renamed {old} to {new}"),
            (Outcome::Failed { .. }, _, Some((old, temp))) => {
                format!("{line} ({old} left as {temp})")
            }
            _ => line,
        }
    }
}

fn source_paths(dir: &Path, names: &[OsString]) -> Sources {
    Sources::Paths(names.iter().map(|name| dir.join(name)).collect())
}

// The commands that require_marks keeps from acting on the cursor row alone:
// those that move or remove what they act on. The batch renames are not among
// them, since their list shows what is about to happen before Enter.
fn requires_marks(command: &Command) -> bool {
    matches!(
        command,
        Command::Copy | Command::Move | Command::Delete | Command::Trash
    )
}

// The commands that work on the entries of an archive without changing it;
// `extract` unpacks the entries that are archives into the other pane.
// `calculate_size` and `calculate_size_on_disk` would not change it either,
// but they only add up files on disk. `copy_files` would need the entries
// extracted first, and putting something on the clipboard, as `copy_path`
// does, should not involve waiting, password prompts, or temporary files.
fn allowed_in_archive(command: &Command) -> bool {
    matches!(
        command,
        Command::OpenWithOs
            | Command::Copy
            | Command::Edit
            | Command::CopyPath
            | Command::CopyName
            | Command::OpenAsArchive
            | Command::Extract
    )
}

// Everything but the extension, so that typing replaces the name and keeps the extension.
fn rename_selection(entry: &Entry) -> Option<(usize, usize)> {
    entry.stem_len().map(|len| (0, len))
}

// The first path component of `path` below `base`, which is where the cursor
// goes after `new_dir` makes `a/b/c`.
fn first_component_under(path: &Path, base: &Path) -> Option<OsString> {
    let rest = path.strip_prefix(base).ok()?;
    match rest.components().next() {
        Some(Component::Normal(name)) => Some(name.to_owned()),
        _ => None,
    }
}

fn file_below(base: &Path, text: &str, os: Os) -> Result<PathBuf, NameProblem> {
    match name_problem(text, os) {
        Some(problem) => Err(problem),
        None => Ok(base.join(text)),
    }
}

// Each part has to be a name, so that neither `..` nor a drive takes the
// directory out of `base`. Empty parts and `.` are skipped as the OS would,
// except that a leading separator means a root.
fn dir_below(base: &Path, text: &str, os: Os) -> Result<PathBuf, NameProblem> {
    let separators: &[char] = if os == Os::Windows {
        &['/', '\\']
    } else {
        &['/']
    };
    if text.starts_with(separators) {
        return Err(NameProblem::Invalid);
    }
    text.split(separators)
        .filter(|part| !part.is_empty() && *part != ".")
        .try_fold(base.to_path_buf(), |path, part| {
            match name_problem(part, os) {
                Some(problem) => Err(problem),
                None => Ok(path.join(part)),
            }
        })
}
