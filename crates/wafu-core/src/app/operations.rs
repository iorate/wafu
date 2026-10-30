use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use super::{App, Change, Effect, MenuAnchor, Os, Side};
use crate::command::{Command, Context};
use crate::entry::Entry;
use crate::error::{AppError, IoError};
use crate::external::Expansion;
use crate::key::{Key, KeyBase, NamedKey};
use crate::location::{ArchiveFormat, Location};
use crate::operation::{
    ArchiveEntries, ArchiveOutput, OperationEvent, OperationKind, OperationSpec, Outcome,
    OutcomeLine, Reply, Sources,
};
use crate::pane::HistoryMove;
use crate::popup::{
    ChoiceKind, ChoicePopup, ConflictPopup, InputKind, InputPopup, PasswordPurpose, Popup,
};
use crate::view::PopupView;

/// The name of an archive made from several entries when the directory has no name.
const DEFAULT_ARCHIVE_STEM: &str = "archive";

/// A file operation in progress on the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Operation {
    kind: OperationKind,
    /// The pane the operation started from, whose marks it clears.
    side: Side,
    /// The names of the sources, in the same order.
    names: Vec<OsString>,
    /// The directory of the sources, or the archive they are in.
    source_dir: PathBuf,
    dest: Option<PathBuf>,
    current: Option<PathBuf>,
    done: usize,
    /// `Cancel` was sent; waiting for the worker to finish.
    cancelling: bool,
    /// Quit once the operation has finished.
    quit_after: bool,
    /// What to do with the entries once they are in the temporary directory.
    temp: Option<Temp>,
    temp_dir: Option<PathBuf>,
    /// A password typed for this operation is kept by the pane, which is
    /// inside the archive.
    remember_password: bool,
}

/// A program to hand the extracted entries to, with the entries each
/// placeholder stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Temp {
    action: TempAction,
    cursor: Option<OsString>,
    marked: Vec<OsString>,
    /// The entries `{paths}` stands for.
    targets: Vec<OsString>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TempAction {
    Edit,
    OpenWithOs,
    External(String),
}

impl App {
    pub(super) fn run_file_command(&mut self, command: Command, effects: &mut Vec<Effect>) {
        if self.active_pane().location().in_archive() && !allowed_in_archive(&command) {
            self.log.push(AppError::InArchive { command }.to_string());
            return;
        }
        if self.config.general.require_marks
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
            Command::Trash => {
                let dir = self.active_pane().location().base.clone();
                let names = self.target_names();
                self.start_removal(OperationKind::Trash, dir, names, effects);
            }
            Command::Rename => self.open_rename(),
            Command::Mkdir => self.open_input(InputKind::Mkdir),
            Command::NewFile => self.open_input(InputKind::NewFile),
            Command::NewText => self.open_input(InputKind::NewText),
            Command::Edit => self.edit(effects),
            Command::CopyPath => self.copy_to_clipboard(true, effects),
            Command::CopyName => self.copy_to_clipboard(false, effects),
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
            Command::EnterAsArchive => self.enter_as_archive(effects),
            Command::Extract => self.start_extract(effects),
            Command::Compress => self.open_compress(),
            _ => {}
        }
    }

    /// Asks for the shell menu of the targets. From the keyboard the targets
    /// follow the usual rule; a right click takes the marked entries only when
    /// the clicked row is one of them, else just that row.
    pub(super) fn shell_menu(&mut self, anchor: MenuAnchor, effects: &mut Vec<Effect>) {
        let pane = self.active_pane();
        if pane.location().in_archive() {
            let command = Command::ShellMenu;
            self.log.push(AppError::InArchive { command }.to_string());
            return;
        }
        let entries = match anchor {
            MenuAnchor::Pointer => match pane.cursor_entry() {
                Some(entry) if pane.is_marked(entry) => pane.targets(),
                Some(entry) => vec![entry],
                None => Vec::new(),
            },
            MenuAnchor::Row { .. } => pane.targets(),
        };
        let base = &pane.location().base;
        let paths = if entries.is_empty() {
            if !pane.has_parent() {
                return;
            }
            vec![base.clone()]
        } else {
            entries.iter().map(|entry| base.join(&entry.name)).collect()
        };
        effects.push(Effect::ShellMenu { paths, anchor });
    }

    fn start_transfer(&mut self, kind: OperationKind, effects: &mut Vec<Effect>) {
        let names = self.target_names();
        if names.is_empty() {
            return;
        }
        let Some(dest) = self.other_dest(kind.noun()) else {
            return;
        };
        let location = self.active_pane().location();
        let source_dir = location.base.clone();
        if location.in_archive() {
            let entries = self.archive_entries(location, &names);
            let sources = Sources::Entries(entries);
            self.start_operation(kind, source_dir, names, sources, Some(dest), effects);
        } else {
            let sources = source_paths(&source_dir, &names);
            self.start_operation(kind, source_dir, names, sources, Some(dest), effects);
        }
    }

    fn start_extract(&mut self, effects: &mut Vec<Effect>) {
        let names = self.target_names();
        if names.is_empty() {
            return;
        }
        let Some(dest) = self.other_dest("extract") else {
            return;
        };
        let source_dir = self.active_pane().location().base.clone();
        let sources = source_paths(&source_dir, &names);
        let kind = OperationKind::Extract;
        self.start_operation(kind, source_dir, names, sources, Some(dest), effects);
    }

    fn open_compress(&mut self) {
        let names = self.target_names();
        if names.is_empty() {
            return;
        }
        let Some(dest) = self.other_dest("compress") else {
            return;
        };
        let dir = self.active_pane().location().base.clone();
        let stem = match names.as_slice() {
            [name] => name.to_string_lossy().into_owned(),
            _ => dir.file_name().map_or_else(
                || DEFAULT_ARCHIVE_STEM.to_owned(),
                |name| name.to_string_lossy().into_owned(),
            ),
        };
        let initial = format!("{stem}.{}", ArchiveFormat::Zip.extension());
        self.popup = Some(Popup::Input(InputPopup::compress(
            dir, dest, names, initial,
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
        let output = ArchiveOutput {
            format: ArchiveFormat::from_name(OsStr::new(name)).unwrap_or(ArchiveFormat::Zip),
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
        let names = self.target_names();
        if !names.is_empty() {
            let dir = self.active_pane().location().base.clone();
            self.popup = Some(Popup::Choice(ChoicePopup::delete_confirm(dir, names)));
        }
    }

    /// Starts a delete or trash. The directory and names come from the
    /// confirmation when there was one, so that what was asked about is what goes.
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

    pub(super) fn confirm_quit(&mut self) {
        if let Some(Popup::Choice(ChoicePopup {
            kind: ChoiceKind::QuitDuringOperation { .. },
            ..
        })) = self.popup
        {
            return;
        }
        let resume = self.popup.take();
        self.popup = Some(Popup::Choice(ChoicePopup::quit_during_operation(resume)));
    }

    /// Chooses the first answer of a quit confirmation.
    pub(super) fn continue_operation(&mut self, resume: Option<Box<Popup>>) {
        self.popup = resume.map(|popup| *popup);
    }

    pub(super) fn cancel_and_quit(&mut self, effects: &mut Vec<Effect>) {
        if let Some(operation) = &mut self.operation {
            operation.quit_after = true;
        }
        self.cancel_operation(effects);
    }

    // Keys while an operation runs: a popup of the operation takes them, and
    // otherwise only Esc and the key for a new window do anything.
    pub(super) fn operation_key(&mut self, key: Key, effects: &mut Vec<Effect>) {
        let is_new_window =
            self.config.keys.lookup(Context::FileList, &key) == Some(Command::NewWindow);
        if is_new_window && !matches!(self.popup, Some(Popup::Input(_))) {
            self.new_window(effects);
            return;
        }
        if self.popup.is_some() {
            self.popup_key(key, effects);
        } else if key.base == KeyBase::Named(NamedKey::Escape) {
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

    /// Answers a password request. An empty password keeps the question open.
    pub(super) fn confirm_password(&mut self, popup: InputPopup, effects: &mut Vec<Effect>) {
        let password = popup.text(0).to_owned();
        if password.is_empty() {
            self.popup = Some(Popup::Input(popup));
            return;
        }
        if let Some(operation) = &self.operation
            && operation.remember_password
        {
            self.panes[operation.side.index()].set_archive_password(Some(password.clone()));
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
                if let Some(name) = operation.names.get(index) {
                    self.panes[operation.side.index()].unmark(name);
                }
            }
            OperationEvent::Conflict { source, dest } => {
                self.show_operation_popup(Popup::Conflict(ConflictPopup::new(source, dest, false)));
            }
            OperationEvent::NeedPassword { archive, retry } => {
                if retry && operation.remember_password {
                    self.panes[operation.side.index()].set_archive_password(None);
                }
                self.show_operation_popup(Popup::Input(InputPopup::password(
                    archive,
                    retry,
                    PasswordPurpose::Operation,
                )));
            }
            OperationEvent::TempDir(path) => operation.temp_dir = Some(path),
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
            message: Some(format!("{} done: {current}", operation.done)),
            progress: true,
            ..PopupView::default()
        })
    }

    pub(super) fn open_rename(&mut self) {
        let pane = self.active_pane();
        if pane.marked_entries().next().is_some() {
            return;
        }
        let Some(entry) = pane.cursor_entry() else {
            return;
        };
        let dir = pane.location().base.clone();
        let original = entry.name.clone();
        let text = original.to_string_lossy().into_owned();
        self.popup = Some(Popup::Input(InputPopup::single(
            InputKind::Rename { dir, original },
            text,
            rename_selection(entry),
        )));
    }

    pub(super) fn confirm_name(&mut self, kind: InputKind, text: &str, effects: &mut Vec<Effect>) {
        if text.is_empty() {
            return;
        }
        let base = match &kind {
            InputKind::Rename { dir, .. } => dir.clone(),
            _ => self.active_pane().location().base.clone(),
        };
        if base.as_os_str().is_empty() {
            return;
        }
        let path = base.join(text);
        match kind {
            InputKind::Rename { original, .. } => {
                if original.to_string_lossy() != text {
                    effects.push(Effect::Rename {
                        from: base.join(&original),
                        to: path,
                    });
                }
            }
            InputKind::Mkdir => effects.push(Effect::Mkdir(path)),
            InputKind::NewFile => effects.push(Effect::CreateFile(path)),
            // Some editors do nothing when given a path that does not exist
            // yet (`open -t` on macOS), so the file is made first by default.
            InputKind::NewText if self.config.general.new_text_creates_file => {
                self.edit_after_create = Some(path.clone());
                effects.push(Effect::CreateFile(path));
            }
            InputKind::NewText => {
                let expansion = Expansion {
                    paths: vec![path],
                    ..self.expansion()
                };
                Self::spawn(&self.config.general.editor, &expansion, effects);
            }
            InputKind::ConflictRename { .. }
            | InputKind::Filter
            | InputKind::MarkGlob
            | InputKind::GoToPath
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
                    "renamed {} to {}",
                    from.file_name().unwrap_or_default().to_string_lossy(),
                    to.file_name().unwrap_or_default().to_string_lossy()
                ));
                if let Some(dir) = to.parent() {
                    self.show_created(dir, to.file_name().map(OsString::from), effects);
                }
            }
            (Change::Renamed { from, .. }, Some(source)) => {
                self.log
                    .push(AppError::RenameFailed { path: from, source }.to_string());
            }
            (Change::DirCreated(path), None) => {
                self.log.push(format!("created {}", path.display()));
                if self.config.general.mkdir_then_enter {
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
                self.log.push(format!("created {}", path.display()));
                if self
                    .edit_after_create
                    .take()
                    .is_some_and(|edit| edit == path)
                {
                    let expansion = Expansion {
                        paths: vec![path.clone()],
                        ..self.expansion()
                    };
                    Self::spawn(&self.config.general.editor, &expansion, effects);
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
        if self.active_pane().location().in_archive() {
            let names = self.target_names();
            let temp = Temp {
                action: TempAction::Edit,
                cursor: None,
                marked: Vec::new(),
                targets: names.clone(),
            };
            let entries = self.archive_entries(self.active_pane().location(), &names);
            self.start_temp(temp, entries, effects);
            return;
        }
        let expansion = self.expansion();
        if expansion.paths.is_empty() {
            return;
        }
        Self::spawn(&self.config.general.editor, &expansion, effects);
    }

    pub(super) fn open_with_os(&mut self, effects: &mut Vec<Effect>) {
        let pane = self.active_pane();
        let location = pane.location().clone();
        match pane.cursor_entry() {
            Some(entry) if location.in_archive() => {
                let name = entry.name.clone();
                let entries = self.archive_entries(&location, std::slice::from_ref(&name));
                self.start_temp(Self::temp_open(name), entries, effects);
            }
            Some(entry) => effects.push(Effect::OpenWithOs(location.base.join(&entry.name))),
            None if pane.has_parent() => match location.parent() {
                Some(parent) if parent.in_archive() => {
                    let name = location
                        .leaf_name()
                        .expect("a directory inside an archive has a name");
                    let entries = self.archive_entries(&parent, std::slice::from_ref(&name));
                    self.start_temp(Self::temp_open(name), entries, effects);
                }
                _ => effects.push(Effect::OpenWithOs(location.base)),
            },
            None => {}
        }
    }

    /// Runs an external command on entries inside an archive, once they are
    /// extracted. Every placeholder may be used, so the cursor entry and the
    /// marked entries are all extracted.
    pub(super) fn run_external_in_archive(&mut self, name: &str, effects: &mut Vec<Effect>) {
        let pane = self.active_pane();
        let cursor = pane.cursor_entry().map(|entry| entry.name.clone());
        let marked: Vec<OsString> = pane
            .marked_entries()
            .map(|entry| entry.name.clone())
            .collect();
        let targets = self.target_names();
        let mut names = marked.clone();
        if let Some(cursor) = &cursor
            && !names.contains(cursor)
        {
            names.push(cursor.clone());
        }
        if names.is_empty() {
            let expansion = Expansion {
                dir: pane.location().dir().to_path_buf(),
                other_dir: self
                    .pane(self.active.other())
                    .location()
                    .dir()
                    .to_path_buf(),
                ..Expansion::default()
            };
            if let Some(line) = self.config.commands.get(name) {
                Self::spawn(line, &expansion, effects);
            }
            return;
        }
        let temp = Temp {
            action: TempAction::External(name.to_owned()),
            cursor,
            marked,
            targets,
        };
        let entries = self.archive_entries(self.active_pane().location(), &names);
        self.start_temp(temp, entries, effects);
    }

    pub(super) fn copy_to_clipboard(&mut self, full_paths: bool, effects: &mut Vec<Effect>) {
        let pane = self.active_pane();
        let location = pane.location();
        let lines: Vec<String> = pane
            .targets()
            .into_iter()
            .map(|entry| {
                if full_paths {
                    location.child_display(&entry.name)
                } else {
                    entry.name.to_string_lossy().into_owned()
                }
            })
            .collect();
        if !lines.is_empty() {
            effects.push(Effect::Clipboard(lines.join("\n")));
        }
    }

    fn target_names(&self) -> Vec<OsString> {
        self.active_pane()
            .targets()
            .into_iter()
            .map(|entry| entry.name.clone())
            .collect()
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

    fn temp_open(name: OsString) -> Temp {
        Temp {
            action: TempAction::OpenWithOs,
            cursor: Some(name.clone()),
            marked: Vec::new(),
            targets: vec![name],
        }
    }

    fn archive_entries(&self, location: &Location, names: &[OsString]) -> ArchiveEntries {
        ArchiveEntries {
            archive: location.base.clone(),
            path: location
                .archive
                .clone()
                .expect("archive entries are only made inside an archive"),
            names: names
                .iter()
                .map(|name| name.to_string_lossy().into_owned())
                .collect(),
            password: self.active_pane().archive_password().map(ToOwned::to_owned),
        }
    }

    fn start_temp(&mut self, temp: Temp, entries: ArchiveEntries, effects: &mut Vec<Effect>) {
        let names = entries.names.iter().map(OsString::from).collect();
        let source_dir = self.active_pane().location().base.clone();
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
        let remember_password = matches!(sources, Sources::Entries(_));
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
            remember_password,
        });
    }

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
        let completed = matches!(outcome, Outcome::Completed { .. });
        if !(completed && operation.temp.is_some()) {
            self.log.push(
                OutcomeLine {
                    kind: operation.kind,
                    outcome,
                }
                .to_string(),
            );
        }
        self.refresh_showing(&operation.source_dir, effects);
        if let Some(dest) = &operation.dest
            && *dest != operation.source_dir
        {
            self.refresh_showing(dest, effects);
        }
        if completed
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
        let to_temp = |name: &OsString| dir.join(name);
        let expansion = Expansion {
            cursor_path: temp.cursor.as_ref().map(to_temp),
            marked_paths: temp.marked.iter().map(to_temp).collect(),
            paths: temp.targets.iter().map(to_temp).collect(),
            dir: self.active_pane().location().dir().to_path_buf(),
            other_dir: self
                .pane(self.active.other())
                .location()
                .dir()
                .to_path_buf(),
        };
        match temp.action {
            TempAction::Edit => Self::spawn(&self.config.general.editor, &expansion, effects),
            TempAction::OpenWithOs => {
                if let Some(path) = expansion.paths.first() {
                    effects.push(Effect::OpenWithOs(path.clone()));
                }
            }
            TempAction::External(name) => match self.config.commands.get(&name) {
                Some(line) => Self::spawn(line, &expansion, effects),
                None => self.log.push(
                    AppError::UnknownCommand {
                        command: Command::External(name),
                    }
                    .to_string(),
                ),
            },
        }
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

fn source_paths(dir: &Path, names: &[OsString]) -> Sources {
    Sources::Paths(names.iter().map(|name| dir.join(name)).collect())
}

// The commands that require_marks keeps from acting on the cursor row alone.
fn requires_marks(command: &Command) -> bool {
    matches!(
        command,
        Command::Copy | Command::Move | Command::Delete | Command::Trash
    )
}

// The commands that work on the entries of an archive without changing it.
fn allowed_in_archive(command: &Command) -> bool {
    matches!(
        command,
        Command::OpenWithOs
            | Command::Copy
            | Command::Edit
            | Command::CopyPath
            | Command::CopyName
            | Command::EnterAsArchive
    )
}

// Everything but the extension, so that typing replaces the name and keeps the extension.
fn rename_selection(entry: &Entry) -> Option<(usize, usize)> {
    let (stem, extension) = entry.split_name();
    (!extension.is_empty()).then_some((0, stem.len()))
}

// The first path component of `path` below `base`, which is where the cursor
// goes after `mkdir a/b/c`.
fn first_component_under(path: &Path, base: &Path) -> Option<OsString> {
    let rest = path.strip_prefix(base).ok()?;
    match rest.components().next() {
        Some(Component::Normal(name)) => Some(name.to_owned()),
        _ => None,
    }
}
