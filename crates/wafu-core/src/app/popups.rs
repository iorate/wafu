use std::mem::{Discriminant, discriminant};

use super::{App, Effect, MouseInput};
use crate::command::Context;
use crate::error::AppError;
use crate::filter::Filter;
use crate::key::{Key, KeyBase, NamedKey};
use crate::location::Location;
use crate::operation::{OperationKind, Reply};
use crate::pane::HistoryMove;
use crate::path::{expand_tilde, resolve_input};
use crate::popup::{
    ChoiceKind, ConflictPopup, ConflictRow, InputKind, InputPopup, ListItem, ListPopup, ListTarget,
    MenuInput, PalettePopup, PasswordPurpose, Popup, Response, SearchPopup, SortPopup, menu_input,
    stem_selection,
};
use crate::view::{InputField, ItemKind, PopupItem, PopupView};

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
            Some(Popup::Input(popup)) => self.input_popup_key(popup, key, effects),
            Some(Popup::Search(popup)) => self.search_popup_key(popup, key),
            Some(popup) => self.menu(popup, MenuInput::Key(key), effects),
            None => {}
        }
    }

    pub(super) fn popup_mouse(&mut self, input: MouseInput, effects: &mut Vec<Effect>) {
        let input = match input {
            MouseInput::PopupClick { row } => MenuInput::Click { row },
            MouseInput::PopupDoubleClick { row } => MenuInput::DoubleClick { row },
            MouseInput::PopupWheel { rows } => {
                self.scroll_popup(rows);
                return;
            }
            _ => return,
        };
        if let Some(popup) = self.popup.take() {
            self.menu(popup, input, effects);
        }
    }

    pub(super) fn set_popup_rows(&mut self, rows: usize) {
        self.popup_rows = rows.max(1);
        self.show_popup_cursor();
    }

    pub(super) fn popup_shape(&self) -> Option<(Discriminant<Popup>, usize, usize)> {
        self.popup
            .as_ref()
            .map(|popup| (discriminant(popup), popup.list_cursor(), popup.list_rows()))
    }

    pub(super) fn follow_popup(&mut self, before: Option<(Discriminant<Popup>, usize, usize)>) {
        let after = self.popup_shape();
        if after == before {
            return;
        }
        if before.map(|(kind, _, _)| kind) != after.map(|(kind, _, _)| kind) {
            self.popup_top = 0;
        }
        self.show_popup_cursor();
    }

    pub(super) fn input_changed(&mut self, field: usize, text: String) {
        match &mut self.popup {
            Some(Popup::Input(popup)) => popup.set_text(field, text),
            Some(Popup::Palette(popup)) if field == 0 => {
                popup.set_query(text);
                self.popup_top = 0;
            }
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
        let view = match popup {
            Popup::Sort(popup) => PopupView {
                title: "Sort".to_owned(),
                items: Some(popup.items()),
                cursor: Some(popup.cursor),
                ..PopupView::default()
            },
            Popup::Input(popup) => PopupView {
                title: popup.kind.title().to_owned(),
                message: popup.message(),
                fields: popup.fields.clone(),
                note: popup.note(),
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
                items: Some(
                    popup
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
                            kind: ItemKind::Choice,
                        })
                        .collect(),
                ),
                cursor: popup.selected().map(|_| popup.cursor),
                ..PopupView::default()
            },
            Popup::List(popup) => PopupView {
                title: popup.title().to_owned(),
                note: popup
                    .items
                    .is_empty()
                    .then(|| popup.empty_message().to_owned()),
                items: (!popup.items.is_empty()).then(|| popup.items()),
                cursor: popup.selected().map(|_| popup.cursor),
                ..PopupView::default()
            },
            Popup::Choice(popup) => PopupView {
                title: popup.title().to_owned(),
                message: Some(popup.message.clone()),
                items: Some(popup.items()),
                cursor: Some(popup.cursor),
                ..PopupView::default()
            },
            Popup::Conflict(popup) => PopupView {
                title: "File exists".to_owned(),
                message: Some(popup.dest.display().to_string()),
                items: Some(popup.items()),
                cursor: Some(popup.cursor),
                ..PopupView::default()
            },
        };
        Some(PopupView {
            scroll_top: self.popup_top,
            ..view
        })
    }

    fn show_popup_cursor(&mut self) {
        let Some(popup) = &self.popup else {
            self.popup_top = 0;
            return;
        };
        let cursor = popup.list_cursor();
        if cursor < self.popup_top {
            self.popup_top = cursor;
        } else if cursor >= self.popup_top + self.popup_rows {
            self.popup_top = cursor + 1 - self.popup_rows;
        }
        self.popup_top = self
            .popup_top
            .min(popup.list_rows().saturating_sub(self.popup_rows));
    }

    fn scroll_popup(&mut self, rows: i32) {
        let Some(popup) = &self.popup else {
            return;
        };
        let top = i64::try_from(self.popup_top).unwrap_or(i64::MAX) + i64::from(rows);
        self.popup_top = usize::try_from(top)
            .unwrap_or(0)
            .min(popup.list_rows().saturating_sub(self.popup_rows));
    }

    fn loading_view(&self) -> Option<PopupView> {
        let targets: Vec<String> = self
            .navigation_target()
            .into_iter()
            .chain(self.preview_loading_name())
            .collect();
        let parts: Vec<String> = self
            .listing_drives()
            .then(|| "Listing drives".to_owned())
            .into_iter()
            .chain((!targets.is_empty()).then(|| format!("Reading {}", targets.join(", "))))
            .collect();
        (!parts.is_empty()).then(|| PopupView {
            title: "Loading".to_owned(),
            message: Some(parts.join(", ")),
            progress: true,
            ..PopupView::default()
        })
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

    fn menu(&mut self, popup: Popup, input: MenuInput, effects: &mut Vec<Effect>) {
        match popup {
            Popup::Sort(mut popup) => match menu_input(&mut popup, input) {
                Response::Stay => self.popup = Some(Popup::Sort(popup)),
                Response::Pick(sort) => self.active_pane_mut().set_sort(sort),
                Response::Close => {}
            },
            Popup::Choice(mut popup) => {
                let response = menu_input(&mut popup, input);
                if response == Response::Stay {
                    self.popup = Some(Popup::Choice(popup));
                    return;
                }
                let confirmed = response == Response::Pick(0);
                match popup.kind {
                    ChoiceKind::DeleteConfirm { dir, names } => {
                        if confirmed {
                            self.start_removal(OperationKind::Delete, dir, names, effects);
                        }
                    }
                    ChoiceKind::QuitDuringOperation { resume } => {
                        if confirmed {
                            self.cancel_and_quit(effects);
                        } else {
                            self.continue_operation(resume);
                        }
                    }
                }
            }
            Popup::Conflict(mut popup) => match menu_input(&mut popup, input) {
                Response::Stay => self.popup = Some(Popup::Conflict(popup)),
                Response::Pick(row) => self.answer_conflict(popup, row, effects),
                Response::Close => self.reply(Reply::Cancel, effects),
            },
            Popup::List(mut popup) => match menu_input(&mut popup, input) {
                Response::Stay => self.popup = Some(Popup::List(popup)),
                Response::Pick(item) => self.pick_list_item(item, effects),
                Response::Close => {}
            },
            Popup::Palette(mut popup) => match menu_input(&mut popup, input) {
                Response::Stay => self.popup = Some(Popup::Palette(popup)),
                Response::Pick(command) => self.run_command(command, effects),
                Response::Close => {}
            },
            Popup::Input(_) | Popup::Search(_) => self.popup = Some(popup),
        }
    }

    fn answer_conflict(
        &mut self,
        popup: ConflictPopup,
        row: ConflictRow,
        effects: &mut Vec<Effect>,
    ) {
        let all = popup.all;
        match row {
            ConflictRow::Overwrite => self.reply(Reply::Overwrite { all }, effects),
            ConflictRow::Skip => self.reply(Reply::Skip { all }, effects),
            ConflictRow::Rename => {
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
            ConflictRow::Abort => self.reply(Reply::Cancel, effects),
            ConflictRow::Separator | ConflictRow::ApplyToAll => {
                self.popup = Some(Popup::Conflict(popup));
            }
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

    fn pick_list_item(&mut self, item: ListItem, effects: &mut Vec<Effect>) {
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
