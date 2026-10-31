use super::{App, Effect};
use crate::command::Context;
use crate::error::AppError;
use crate::filter::Filter;
use crate::key::{Key, KeyBase, NamedKey};
use crate::location::Location;
use crate::operation::{OperationKind, Reply};
use crate::pane::HistoryMove;
use crate::path::{expand_tilde, resolve_input};
use crate::popup::{
    ChoiceKind, ChoicePopup, ConflictPopup, ConflictRow, InputKind, InputPopup, ListItem,
    ListPopup, ListTarget, PalettePopup, PasswordPurpose, Popup, SearchPopup, SortPopup,
    stem_selection,
};
use crate::view::{InputField, ItemMark, PopupItem, PopupView};

impl App {
    pub(super) fn open_sort(&mut self) {
        let sort = self.active_pane().sort();
        self.popup = Some(Popup::Sort(SortPopup::new(sort)));
    }

    pub(super) fn open_input(&mut self, kind: InputKind) {
        let pane = self.active_pane();
        let text = match kind {
            InputKind::Filter => pane
                .filter()
                .map(|filter| filter.pattern().to_owned())
                .unwrap_or_default(),
            InputKind::GoToPath => pane.location().base.to_string_lossy().into_owned(),
            InputKind::MarkGlob
            | InputKind::Rename { .. }
            | InputKind::Mkdir
            | InputKind::NewFile
            | InputKind::NewText
            | InputKind::ConflictRename { .. }
            | InputKind::ArchivePassword { .. }
            | InputKind::Compress { .. } => String::new(),
        };
        self.popup = Some(Popup::Input(InputPopup::single(kind, text, None)));
    }

    pub(super) fn open_search(&mut self) {
        let pane = self.active_pane();
        self.popup = Some(Popup::Search(SearchPopup {
            query: String::new(),
            origin_cursor: pane.cursor(),
            origin_scroll_top: pane.scroll_top(),
        }));
    }

    pub(super) fn open_palette(&mut self) {
        let externals = self.config.commands.keys().cloned().collect();
        self.popup = Some(Popup::Palette(PalettePopup::new(
            Context::FileList,
            self.os,
            externals,
        )));
    }

    pub(super) fn open_history(&mut self) {
        let popup = ListPopup::history(self.active_pane().history());
        self.popup = Some(Popup::List(popup));
    }

    pub(super) fn open_bookmarks(&mut self) {
        let paths = self
            .config
            .bookmarks
            .iter()
            .map(|path| expand_tilde(&path.to_string_lossy(), &self.home))
            .collect();
        self.popup = Some(Popup::List(ListPopup::bookmarks(paths)));
    }

    pub(super) fn open_drives(&mut self, letters: &[char]) {
        let popup = ListPopup::drives(letters, self.drive_of_active());
        self.popup = Some(Popup::List(popup));
    }

    // The popup is taken out first and put back unless the key closed it.
    pub(super) fn popup_key(&mut self, key: Key, effects: &mut Vec<Effect>) {
        match self.popup.take() {
            Some(Popup::Sort(popup)) => self.sort_popup_key(popup, key),
            Some(Popup::Input(popup)) => self.input_popup_key(popup, key, effects),
            Some(Popup::Search(popup)) => self.search_popup_key(popup, key),
            Some(Popup::Palette(popup)) => self.palette_popup_key(popup, key, effects),
            Some(Popup::List(popup)) => self.list_popup_key(popup, key, effects),
            Some(Popup::Choice(popup)) => self.choice_popup_key(popup, key, effects),
            Some(Popup::Conflict(popup)) => self.conflict_popup_key(popup, key, effects),
            None => {}
        }
    }

    pub(super) fn input_changed(&mut self, field: usize, text: String) {
        match &mut self.popup {
            Some(Popup::Input(popup)) => popup.set_text(field, text),
            Some(Popup::Palette(popup)) if field == 0 => popup.set_query(text),
            Some(Popup::Search(popup)) if field == 0 => {
                let pane = &mut self.panes[self.active.index()];
                if text.is_empty() {
                    pane.restore_position(popup.origin_cursor, popup.origin_scroll_top);
                    popup.query = text;
                } else if pane.search_first(&text) {
                    popup.query = text;
                }
                // Otherwise the text is refused and the view keeps the old query.
            }
            Some(
                Popup::Sort(_)
                | Popup::List(_)
                | Popup::Choice(_)
                | Popup::Conflict(_)
                | Popup::Palette(_)
                | Popup::Search(_),
            )
            | None => {}
        }
    }

    pub(super) fn popup_view(&self) -> Option<PopupView> {
        let Some(popup) = self.popup.as_ref() else {
            return self.operation_view().or_else(|| self.loading_view());
        };
        Some(match popup {
            Popup::Sort(popup) => PopupView {
                title: "Sort".to_owned(),
                items: popup.items(),
                cursor: Some(popup.cursor),
                ..PopupView::default()
            },
            Popup::Input(popup) => PopupView {
                title: popup.kind.title().to_owned(),
                message: popup.message(),
                fields: popup.fields.clone(),
                ..PopupView::default()
            },
            Popup::Search(popup) => PopupView {
                title: "Search".to_owned(),
                fields: vec![InputField::new(popup.query.clone())],
                ..PopupView::default()
            },
            Popup::Palette(popup) => PopupView {
                title: "Palette".to_owned(),
                fields: vec![InputField::new(popup.query.clone())],
                items: popup
                    .matches
                    .iter()
                    .map(|command| PopupItem {
                        label: command.to_string(),
                        detail: self
                            .config
                            .keys
                            .keys_for(popup.context, command)
                            .iter()
                            .map(ToString::to_string)
                            .collect::<Vec<_>>()
                            .join(", "),
                        mark: ItemMark::None,
                    })
                    .collect(),
                cursor: popup.selected().map(|_| popup.cursor),
                ..PopupView::default()
            },
            Popup::List(popup) => PopupView {
                title: popup.title().to_owned(),
                message: popup
                    .items
                    .is_empty()
                    .then(|| popup.empty_message().to_owned()),
                items: popup.items(),
                cursor: popup.selected().map(|_| popup.cursor),
                ..PopupView::default()
            },
            Popup::Choice(popup) => PopupView {
                title: popup.title().to_owned(),
                message: Some(popup.message.clone()),
                items: popup.items(),
                cursor: Some(popup.cursor),
                ..PopupView::default()
            },
            Popup::Conflict(popup) => PopupView {
                title: "File exists".to_owned(),
                message: Some(popup.dest.display().to_string()),
                items: popup.items(),
                cursor: Some(popup.cursor),
                ..PopupView::default()
            },
        })
    }

    fn loading_view(&self) -> Option<PopupView> {
        let targets: Vec<String> = self
            .navigation_target()
            .into_iter()
            .chain(self.preview_loading_name())
            .collect();
        (!targets.is_empty()).then(|| PopupView {
            title: "Loading".to_owned(),
            message: Some(format!("Reading {}", targets.join(", "))),
            progress: true,
            ..PopupView::default()
        })
    }

    fn sort_popup_key(&mut self, mut popup: SortPopup, key: Key) {
        let plain = !key.ctrl && !key.alt;
        match key.base {
            KeyBase::Named(NamedKey::Escape) => return,
            KeyBase::Named(NamedKey::Enter) => {
                self.active_pane_mut().set_sort(popup.draft);
                return;
            }
            KeyBase::Named(NamedKey::Up) => popup.cursor_up(),
            KeyBase::Named(NamedKey::Down) => popup.cursor_down(),
            KeyBase::Named(NamedKey::Space) => popup.activate_cursor(),
            KeyBase::Char(letter) if plain => {
                if let Some(sort_key) = SortPopup::key_for_letter(letter) {
                    popup.draft.key = sort_key;
                    self.active_pane_mut().set_sort(popup.draft);
                    return;
                }
                popup.toggle_for_letter(letter);
            }
            _ => {}
        }
        self.popup = Some(Popup::Sort(popup));
    }

    fn input_popup_key(&mut self, popup: InputPopup, key: Key, effects: &mut Vec<Effect>) {
        match key.base {
            KeyBase::Named(NamedKey::Escape) => match popup.kind {
                // The conflict is still waiting for its answer.
                InputKind::ConflictRename { source, dest, all } => {
                    self.popup = Some(Popup::Conflict(ConflictPopup::new(source, dest, all)));
                }
                // So is the worker, and without a password it cannot go on.
                InputKind::ArchivePassword {
                    purpose: PasswordPurpose::Operation,
                    ..
                } => self.cancel_operation(effects),
                InputKind::ArchivePassword {
                    purpose: PasswordPurpose::Preview,
                    ..
                } => self.preview_password_refused(effects),
                _ => {}
            },
            KeyBase::Named(NamedKey::Enter) => self.confirm_input(popup, effects),
            _ => self.popup = Some(Popup::Input(popup)),
        }
    }

    fn confirm_input(&mut self, popup: InputPopup, effects: &mut Vec<Effect>) {
        let text = popup.text(0).to_owned();
        let password = popup.text(1).to_owned();
        match popup.kind {
            InputKind::GoToPath => {
                let text = text.trim();
                if text.is_empty() {
                    return;
                }
                let base = &self.active_pane().location().base;
                let path = resolve_input(text, &self.home, base);
                self.resolve(self.active, path, false, None, effects);
                return;
            }
            InputKind::Rename { .. }
            | InputKind::Mkdir
            | InputKind::NewFile
            | InputKind::NewText => {
                self.confirm_name(popup.kind, &text, effects);
                return;
            }
            InputKind::ArchivePassword {
                purpose: PasswordPurpose::Operation,
                ..
            } => {
                self.confirm_password(popup, effects);
                return;
            }
            InputKind::ArchivePassword {
                purpose: PasswordPurpose::Preview,
                ..
            } => {
                self.preview_password(popup, effects);
                return;
            }
            InputKind::Compress { dir, dest, names } => {
                self.confirm_compress(dir, dest, names, &text, &password, effects);
                return;
            }
            InputKind::ConflictRename { source, dest, all } => {
                let name = text;
                let same = dest
                    .file_name()
                    .is_some_and(|current| current.to_string_lossy() == name);
                if name.is_empty() || same {
                    self.popup = Some(Popup::Conflict(ConflictPopup::new(source, dest, all)));
                } else {
                    self.reply(Reply::Rename(name.into()), effects);
                }
                return;
            }
            InputKind::Filter | InputKind::MarkGlob => {}
        }
        let filter = match Filter::parse(&text) {
            Ok(filter) => filter,
            Err(source) => {
                self.log.push(
                    AppError::InvalidPattern {
                        pattern: text,
                        source,
                    }
                    .to_string(),
                );
                return;
            }
        };
        let pane = self.active_pane_mut();
        match popup.kind {
            InputKind::Filter => {
                pane.set_filter(filter);
            }
            InputKind::MarkGlob => {
                if let Some(filter) = filter {
                    pane.mark_matching(&filter);
                }
            }
            _ => {}
        }
    }

    fn choice_popup_key(&mut self, mut popup: ChoicePopup, key: Key, effects: &mut Vec<Effect>) {
        let picked = match key.base {
            KeyBase::Named(NamedKey::Escape) => Some(None),
            KeyBase::Named(NamedKey::Enter) => Some(Some(popup.cursor)),
            KeyBase::Named(NamedKey::Up) => {
                popup.cursor_up();
                None
            }
            KeyBase::Named(NamedKey::Down) => {
                popup.cursor_down();
                None
            }
            KeyBase::Char(letter) if !key.ctrl && !key.alt => {
                popup.choice_for_letter(letter).map(Some)
            }
            _ => None,
        };
        let Some(choice) = picked else {
            self.popup = Some(Popup::Choice(popup));
            return;
        };
        match popup.kind {
            ChoiceKind::DeleteConfirm { dir, names } => {
                if choice == Some(0) {
                    self.start_removal(OperationKind::Delete, dir, names, effects);
                }
            }
            ChoiceKind::QuitDuringOperation { resume } => {
                if choice == Some(0) {
                    self.cancel_and_quit(effects);
                } else {
                    self.continue_operation(resume);
                }
            }
        }
    }

    fn conflict_popup_key(
        &mut self,
        mut popup: ConflictPopup,
        key: Key,
        effects: &mut Vec<Effect>,
    ) {
        let row = match key.base {
            KeyBase::Named(NamedKey::Escape) => Some(ConflictRow::Abort),
            KeyBase::Named(NamedKey::Enter) => Some(popup.row_at_cursor()),
            KeyBase::Named(NamedKey::Space) => Some(ConflictRow::ApplyToAll),
            KeyBase::Named(NamedKey::Up) => {
                popup.cursor_up();
                None
            }
            KeyBase::Named(NamedKey::Down) => {
                popup.cursor_down();
                None
            }
            KeyBase::Char(letter) if !key.ctrl && !key.alt => ConflictPopup::row_for_letter(letter),
            _ => None,
        };
        let all = popup.all;
        match row {
            Some(ConflictRow::Overwrite) => self.reply(Reply::Overwrite { all }, effects),
            Some(ConflictRow::Skip) => self.reply(Reply::Skip { all }, effects),
            Some(ConflictRow::Abort) => self.reply(Reply::Cancel, effects),
            Some(ConflictRow::Rename) => {
                let text = popup
                    .dest
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let select = stem_selection(&text);
                self.popup = Some(Popup::Input(InputPopup::single(
                    InputKind::ConflictRename {
                        source: popup.source,
                        dest: popup.dest,
                        all,
                    },
                    text,
                    select,
                )));
            }
            Some(ConflictRow::ApplyToAll) => {
                popup.all = !popup.all;
                self.popup = Some(Popup::Conflict(popup));
            }
            None => self.popup = Some(Popup::Conflict(popup)),
        }
    }

    fn search_popup_key(&mut self, popup: SearchPopup, key: Key) {
        let pane = &mut self.panes[self.active.index()];
        match key.base {
            KeyBase::Named(NamedKey::Escape) => {
                pane.restore_position(popup.origin_cursor, popup.origin_scroll_top);
                return;
            }
            KeyBase::Named(NamedKey::Enter) => return,
            KeyBase::Named(NamedKey::Down) if !popup.query.is_empty() => {
                pane.search_next(&popup.query);
            }
            KeyBase::Named(NamedKey::Up) if !popup.query.is_empty() => {
                pane.search_prev(&popup.query);
            }
            _ => {}
        }
        self.popup = Some(Popup::Search(popup));
    }

    fn palette_popup_key(&mut self, mut popup: PalettePopup, key: Key, effects: &mut Vec<Effect>) {
        match key.base {
            KeyBase::Named(NamedKey::Escape) => return,
            KeyBase::Named(NamedKey::Enter) => {
                if let Some(command) = popup.selected() {
                    self.run_command(command, effects);
                }
                return;
            }
            KeyBase::Named(NamedKey::Up) => popup.cursor_up(),
            KeyBase::Named(NamedKey::Down) => popup.cursor_down(),
            _ => {}
        }
        self.popup = Some(Popup::Palette(popup));
    }

    fn list_popup_key(&mut self, mut popup: ListPopup, key: Key, effects: &mut Vec<Effect>) {
        match key.base {
            KeyBase::Named(NamedKey::Escape) => return,
            KeyBase::Named(NamedKey::Enter) => {
                if let Some(item) = popup.selected() {
                    self.pick_list_item(item.clone(), effects);
                }
                return;
            }
            KeyBase::Named(NamedKey::Up) => popup.cursor_up(),
            KeyBase::Named(NamedKey::Down) => popup.cursor_down(),
            KeyBase::Char(c) if !key.ctrl && !key.alt => {
                if let Some(item) = popup.item_for_key(c) {
                    self.pick_list_item(item.clone(), effects);
                    return;
                }
            }
            _ => {}
        }
        self.popup = Some(Popup::List(popup));
    }

    fn pick_list_item(&mut self, item: ListItem, effects: &mut Vec<Effect>) {
        if item.current {
            return;
        }
        match item.target {
            ListTarget::HistoryPos(pos) => {
                let path = self.active_pane().history().items().get(pos).cloned();
                self.go_to_history(path.map(|path| (pos, path)), effects);
            }
            ListTarget::Path(path) => {
                self.navigate(
                    self.active,
                    Location::new(path),
                    None,
                    HistoryMove::Visit,
                    effects,
                );
            }
            ListTarget::Drive(letter) => self.switch_drive(letter, effects),
        }
    }
}
