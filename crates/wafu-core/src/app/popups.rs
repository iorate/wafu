use super::{App, Effect};
use crate::command::Context;
use crate::error::AppError;
use crate::filter::Filter;
use crate::key::{Key, KeyBase, NamedKey};
use crate::popup::{InputKind, InputPopup, PalettePopup, Popup, SearchPopup, SortPopup};
use crate::view::{ItemMark, PopupItem, PopupView};

impl App {
    pub(super) fn open_sort(&mut self) {
        let sort = self.active_pane().sort();
        self.popup = Some(Popup::Sort(SortPopup::new(sort)));
    }

    pub(super) fn open_input(&mut self, kind: InputKind) {
        let text = match kind {
            InputKind::Filter => self
                .active_pane()
                .filter()
                .map(|filter| filter.pattern().to_owned())
                .unwrap_or_default(),
            InputKind::MarkGlob => String::new(),
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

    // The popup is taken out first and put back unless the key closed it.
    pub(super) fn popup_key(&mut self, key: Key, effects: &mut Vec<Effect>) {
        match self.popup.take() {
            Some(Popup::Sort(popup)) => self.sort_popup_key(popup, key),
            Some(Popup::Input(popup)) => self.input_popup_key(popup, key),
            Some(Popup::Search(popup)) => self.search_popup_key(popup, key),
            Some(Popup::Palette(popup)) => self.palette_popup_key(popup, key, effects),
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
            Some(Popup::Sort(_)) | None => {}
        }
    }

    pub(super) fn popup_view(&self) -> Option<PopupView> {
        Some(match self.popup.as_ref()? {
            Popup::Sort(popup) => PopupView {
                title: "Sort".to_owned(),
                input: None,
                items: popup.items(),
                cursor: Some(popup.cursor),
            },
            Popup::Input(popup) => PopupView {
                title: popup.kind.title().to_owned(),
                input: Some(popup.text.clone()),
                items: Vec::new(),
                cursor: None,
            },
            Popup::Search(popup) => PopupView {
                title: "Search".to_owned(),
                input: Some(popup.query.clone()),
                items: Vec::new(),
                cursor: None,
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
            },
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

    fn input_popup_key(&mut self, popup: InputPopup, key: Key) {
        match key.base {
            KeyBase::Named(NamedKey::Escape) => {}
            KeyBase::Named(NamedKey::Enter) => self.confirm_input(popup),
            _ => self.popup = Some(Popup::Input(popup)),
        }
    }

    fn confirm_input(&mut self, popup: InputPopup) {
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
}
