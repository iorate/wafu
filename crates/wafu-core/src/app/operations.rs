use std::ffi::{OsStr, OsString};
use std::path::{Component, Path, PathBuf};

use super::{App, Change, Effect, Os, Side};
use crate::command::{Command, Context};
use crate::entry::Entry;
use crate::error::{AppError, IoError};
use crate::external::Expansion;
use crate::key::{Key, KeyBase, NamedKey};
use crate::location::Location;
use crate::operation::{OperationEvent, OperationKind, OperationSpec, Outcome, OutcomeLine, Reply};
use crate::pane::{HistoryMove, TargetRule};
use crate::popup::{ChoiceKind, ChoicePopup, ConflictPopup, InputKind, InputPopup, Popup};
use crate::view::PopupView;

/// A file operation in progress on the worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Operation {
    kind: OperationKind,
    /// The pane the operation started from, whose marks it clears.
    side: Side,
    /// The names of the sources, in the same order.
    names: Vec<OsString>,
    source_dir: PathBuf,
    dest: Option<PathBuf>,
    current: Option<PathBuf>,
    done: usize,
    /// `Cancel` was sent; waiting for the worker to finish.
    cancelling: bool,
    /// Quit once the operation has finished.
    quit_after: bool,
}

impl App {
    pub(super) fn run_file_command(&mut self, command: Command, effects: &mut Vec<Effect>) {
        match command {
            Command::OpenWithOs => self.open_with_os(effects),
            Command::Copy => self.start_transfer(OperationKind::Copy, effects),
            Command::Move => self.start_transfer(OperationKind::Move, effects),
            Command::Delete => self.confirm_delete(),
            Command::Trash => self.start_removal(OperationKind::Trash, None, effects),
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
            _ => {}
        }
    }

    fn start_transfer(&mut self, kind: OperationKind, effects: &mut Vec<Effect>) {
        let dest = self.pane(self.active.other()).location().base.clone();
        if dest.as_os_str().is_empty() {
            self.log
                .push(AppError::NoOtherLocation { verb: kind.noun() }.to_string());
            return;
        }
        let names = self.target_names();
        self.start_operation(kind, names, Some(dest), effects);
    }

    pub(super) fn confirm_delete(&mut self) {
        let names = self.target_names();
        if !names.is_empty() {
            self.popup = Some(Popup::Choice(ChoicePopup::delete_confirm(names)));
        }
    }

    /// Starts a delete or trash. The names come from the confirmation when
    /// there was one, so that what was asked about is what goes.
    pub(super) fn start_removal(
        &mut self,
        kind: OperationKind,
        names: Option<Vec<OsString>>,
        effects: &mut Vec<Effect>,
    ) {
        let names = names.unwrap_or_else(|| self.target_names());
        self.start_operation(kind, names, None, effects);
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
                self.popup = Some(Popup::Conflict(ConflictPopup::new(source, dest, false)));
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
        let original = entry.name.clone();
        let text = original.to_string_lossy().into_owned();
        self.popup = Some(Popup::Input(InputPopup {
            kind: InputKind::Rename { original },
            select: rename_selection(entry),
            text,
        }));
    }

    pub(super) fn confirm_name(&mut self, kind: InputKind, text: &str, effects: &mut Vec<Effect>) {
        if text.is_empty() {
            return;
        }
        let base = self.active_pane().location().base.clone();
        if base.as_os_str().is_empty() {
            return;
        }
        let path = base.join(text);
        match kind {
            InputKind::Rename { original } => {
                if OsStr::new(text) != original {
                    effects.push(Effect::Rename {
                        from: base.join(&original),
                        to: path,
                    });
                }
            }
            InputKind::Mkdir => effects.push(Effect::Mkdir(path)),
            InputKind::NewFile => effects.push(Effect::CreateFile(path)),
            // Editors cannot be relied on to open a path that does not exist
            // yet (`open -t` on macOS does nothing), so the file is made first.
            InputKind::NewText => {
                self.edit_after_create = Some(path.clone());
                effects.push(Effect::CreateFile(path));
            }
            InputKind::ConflictRename { .. }
            | InputKind::Filter
            | InputKind::MarkGlob
            | InputKind::GoToPath => {}
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
        let expansion = self.expansion();
        if expansion.paths.is_empty() {
            return;
        }
        Self::spawn(&self.config.general.editor, &expansion, effects);
    }

    pub(super) fn open_with_os(&mut self, effects: &mut Vec<Effect>) {
        let pane = self.active_pane();
        if let Some(entry) = pane.cursor_entry() {
            effects.push(Effect::OpenWithOs(pane.location().base.join(&entry.name)));
        }
    }

    pub(super) fn copy_to_clipboard(&mut self, full_paths: bool, effects: &mut Vec<Effect>) {
        let pane = self.active_pane();
        let base = &pane.location().base;
        let lines: Vec<String> = pane
            .targets(TargetRule::Multiple)
            .into_iter()
            .map(|entry| {
                if full_paths {
                    base.join(&entry.name).to_string_lossy().into_owned()
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
            .targets(TargetRule::Multiple)
            .into_iter()
            .map(|entry| entry.name.clone())
            .collect()
    }

    fn start_operation(
        &mut self,
        kind: OperationKind,
        names: Vec<OsString>,
        dest: Option<PathBuf>,
        effects: &mut Vec<Effect>,
    ) {
        if names.is_empty() {
            return;
        }
        let source_dir = self.active_pane().location().base.clone();
        if source_dir.as_os_str().is_empty() {
            return;
        }
        let sources = names.iter().map(|name| source_dir.join(name)).collect();
        effects.push(Effect::StartOperation(OperationSpec {
            kind,
            sources,
            dest: dest.clone(),
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
        });
    }

    fn finished(&mut self, outcome: &Outcome, effects: &mut Vec<Effect>) {
        let Some(operation) = self.operation.take() else {
            return;
        };
        self.popup = None;
        self.log.push(
            OutcomeLine {
                kind: operation.kind,
                outcome,
            }
            .to_string(),
        );
        self.refresh_showing(&operation.source_dir, effects);
        if let Some(dest) = &operation.dest
            && *dest != operation.source_dir
        {
            self.refresh_showing(dest, effects);
        }
        if operation.quit_after {
            self.quit(true, effects);
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

// Everything but the extension, so that typing replaces the name and keeps the extension.
fn rename_selection(entry: &Entry) -> Option<(usize, usize)> {
    let (stem, extension) = entry.split_name();
    (!extension.is_empty()).then_some((0, stem.len()))
}

/// The part of a file name to select when renaming it: everything but the
/// extension, or nothing special when there is none.
pub(super) fn stem_selection(name: &str) -> Option<(usize, usize)> {
    name.char_indices()
        .skip(1)
        .filter(|&(_, c)| c == '.')
        .map(|(index, _)| index)
        .last()
        .map(|index| (0, index))
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
