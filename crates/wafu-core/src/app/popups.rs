use super::{App, Effect};
use crate::command::Context;
use crate::error::AppError;
use crate::filter::Filter;
use crate::key::{Key, KeyBase, NamedKey};
use crate::location::Location;
use crate::pane::HistoryMove;
use crate::path::{expand_tilde, resolve_input};
use crate::popup::{
    InputKind, InputPopup, ListItem, ListPopup, ListTarget, PalettePopup, Popup, SearchPopup,
    SortPopup,
};
use crate::view::{ItemMark, PopupItem, PopupView};

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
            InputKind::MarkGlob => String::new(),
            InputKind::GoToPath => pane.location().base.to_string_lossy().into_owned(),
        };
        self.popup = Some(Popup::Input(InputPopup { kind, text }));
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
        self.popup = Some(Popup::Palette(PalettePopup::new(Context::FileList)));
    }

    pub(super) fn open_history(&mut self) {
        let popup = ListPopup::history(self.active_pane().history());
        self.popup = Some(Popup::List(popup));
    }

    pub(super) fn open_bookmarks(&mut self) {
        let paths = self
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
            None => {}
        }
    }

    pub(super) fn input_changed(&mut self, text: String) {
        match &mut self.popup {
            Some(Popup::Input(popup)) => popup.text = text,
            Some(Popup::Palette(popup)) => popup.set_query(text),
            Some(Popup::Search(popup)) => {
                let pane = &mut self.panes[self.active.index()];
                if text.is_empty() {
                    pane.restore_position(popup.origin_cursor, popup.origin_scroll_top);
                    popup.query = text;
                } else if pane.search_first(&text) {
                    popup.query = text;
                }
                // Otherwise the text is refused and the view keeps the old query.
            }
            Some(Popup::Sort(_) | Popup::List(_)) | None => {}
        }
    }

    pub(super) fn popup_view(&self) -> Option<PopupView> {
        let Some(popup) = self.popup.as_ref() else {
            return self.loading_view();
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
                input: Some(popup.text.clone()),
                ..PopupView::default()
            },
            Popup::Search(popup) => PopupView {
                title: "Search".to_owned(),
                input: Some(popup.query.clone()),
                ..PopupView::default()
            },
            Popup::Palette(popup) => PopupView {
                title: "Palette".to_owned(),
                input: Some(popup.query.clone()),
                items: popup
                    .matches
                    .iter()
                    .map(|command| PopupItem {
                        label: command.name().to_owned(),
                        detail: self
                            .keymap
                            .keys_for(popup.context, *command)
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
        })
    }

    fn loading_view(&self) -> Option<PopupView> {
        let targets = self.navigation_targets();
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
            KeyBase::Named(NamedKey::Escape) => {}
            KeyBase::Named(NamedKey::Enter) => self.confirm_input(popup, effects),
            _ => self.popup = Some(Popup::Input(popup)),
        }
    }

    fn confirm_input(&mut self, popup: InputPopup, effects: &mut Vec<Effect>) {
        if popup.kind == InputKind::GoToPath {
            let text = popup.text.trim();
            if text.is_empty() {
                return;
            }
            let base = &self.active_pane().location().base;
            let target = Location::new(resolve_input(text, &self.home, base));
            self.navigate(self.active, target, None, HistoryMove::Visit, effects);
            return;
        }
        let filter = match Filter::parse(&popup.text) {
            Ok(filter) => filter,
            Err(source) => {
                self.log.push(
                    AppError::InvalidPattern {
                        pattern: popup.text,
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
            InputKind::GoToPath => {}
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
