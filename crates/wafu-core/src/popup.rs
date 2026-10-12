use crate::command::{Command, Context};
use crate::listing::{SortKey, SortSpec};
use crate::view::{ItemMark, PopupItem};

/// A modal that takes all keys while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Popup {
    Sort(SortPopup),
    Input(InputPopup),
    Search(SearchPopup),
    Palette(PalettePopup),
}

/// Edits a copy of the sort settings; nothing is applied until confirmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SortPopup {
    pub draft: SortSpec,
    /// Row in [`SortPopup::items`]: the sort keys first, then the three toggles.
    pub cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputPopup {
    pub kind: InputKind,
    pub text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    Filter,
    MarkGlob,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchPopup {
    pub query: String,
    pub origin_cursor: usize,
    pub origin_scroll_top: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PalettePopup {
    pub context: Context,
    pub query: String,
    pub matches: Vec<Command>,
    pub cursor: usize,
}

const SORT_KEYS: [(SortKey, &str, char); 6] = [
    (SortKey::None, "None", 'N'),
    (SortKey::Name, "Name", 'F'),
    (SortKey::Extension, "Extension", 'E'),
    (SortKey::Size, "Size", 'S'),
    (SortKey::Time, "Time", 'T'),
    (SortKey::Natural, "Name (natural)", 'X'),
];

const SORT_TOGGLES: [(&str, char); 3] = [
    ("Descending", 'D'),
    ("Directories first", 'G'),
    ("Case-insensitive", 'C'),
];

impl SortPopup {
    pub const ROWS: usize = SORT_KEYS.len() + SORT_TOGGLES.len();

    #[must_use]
    pub fn new(sort: SortSpec) -> Self {
        Self {
            draft: sort,
            cursor: SORT_KEYS
                .iter()
                .position(|(key, _, _)| *key == sort.key)
                .unwrap_or(0),
        }
    }

    #[must_use]
    pub fn items(&self) -> Vec<PopupItem> {
        let keys = SORT_KEYS.iter().map(|(key, label, letter)| PopupItem {
            label: (*label).to_owned(),
            detail: letter.to_string(),
            mark: ItemMark::Radio(self.draft.key == *key),
        });
        let toggles = SORT_TOGGLES
            .iter()
            .zip(self.toggle_values())
            .map(|((label, letter), on)| PopupItem {
                label: (*label).to_owned(),
                detail: letter.to_string(),
                mark: ItemMark::Check(on),
            });
        keys.chain(toggles).collect()
    }

    pub fn cursor_up(&mut self) {
        self.cursor = self.cursor.checked_sub(1).unwrap_or(Self::ROWS - 1);
    }

    pub fn cursor_down(&mut self) {
        self.cursor = (self.cursor + 1) % Self::ROWS;
    }

    /// Selects the sort key on the cursor row, or flips the toggle there.
    pub fn activate_cursor(&mut self) {
        if let Some((key, _, _)) = SORT_KEYS.get(self.cursor) {
            self.draft.key = *key;
        } else {
            self.toggle(self.cursor - SORT_KEYS.len());
        }
    }

    /// The sort key selected by a letter, if the letter stands for one.
    #[must_use]
    pub fn key_for_letter(letter: char) -> Option<SortKey> {
        SORT_KEYS
            .iter()
            .find(|(_, _, l)| *l == letter)
            .map(|(key, _, _)| *key)
    }

    /// Flips the toggle a letter stands for. Returns whether the letter was one.
    pub fn toggle_for_letter(&mut self, letter: char) -> bool {
        match SORT_TOGGLES.iter().position(|(_, l)| *l == letter) {
            Some(index) => {
                self.toggle(index);
                true
            }
            None => false,
        }
    }

    fn toggle_values(&self) -> [bool; 3] {
        [
            self.draft.descending,
            self.draft.dirs_first,
            self.draft.ignore_case,
        ]
    }

    fn toggle(&mut self, index: usize) {
        let flag = match index {
            0 => &mut self.draft.descending,
            1 => &mut self.draft.dirs_first,
            _ => &mut self.draft.ignore_case,
        };
        *flag = !*flag;
    }
}

impl InputKind {
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Filter => "Filter",
            Self::MarkGlob => "Mark",
        }
    }
}

impl PalettePopup {
    #[must_use]
    pub fn new(context: Context) -> Self {
        Self {
            context,
            query: String::new(),
            matches: matches(context, ""),
            cursor: 0,
        }
    }

    pub fn set_query(&mut self, query: String) {
        self.matches = matches(self.context, &query);
        self.query = query;
        self.cursor = 0;
    }

    #[must_use]
    pub fn selected(&self) -> Option<Command> {
        self.matches.get(self.cursor).copied()
    }

    pub fn cursor_up(&mut self) {
        if !self.matches.is_empty() {
            self.cursor = self.cursor.checked_sub(1).unwrap_or(self.matches.len() - 1);
        }
    }

    pub fn cursor_down(&mut self) {
        if !self.matches.is_empty() {
            self.cursor = (self.cursor + 1) % self.matches.len();
        }
    }
}

fn matches(context: Context, query: &str) -> Vec<Command> {
    let query = query.to_lowercase();
    Command::all()
        .filter(|command| command.available_in(context))
        .filter(|command| command.name().contains(&query))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_popup_opens_on_the_current_key() {
        let popup = SortPopup::new(SortSpec {
            key: SortKey::Size,
            ..SortSpec::default()
        });

        assert_eq!(popup.cursor, 3);
        let items = popup.items();
        let rows: Vec<(&str, ItemMark)> =
            items.iter().map(|i| (i.label.as_str(), i.mark)).collect();
        assert_eq!(
            rows,
            [
                ("None", ItemMark::Radio(false)),
                ("Name", ItemMark::Radio(false)),
                ("Extension", ItemMark::Radio(false)),
                ("Size", ItemMark::Radio(true)),
                ("Time", ItemMark::Radio(false)),
                ("Name (natural)", ItemMark::Radio(false)),
                ("Descending", ItemMark::Check(false)),
                ("Directories first", ItemMark::Check(true)),
                ("Case-insensitive", ItemMark::Check(true)),
            ]
        );
        assert_eq!(items[5].detail, "X");
    }

    #[test]
    fn sort_popup_cursor_wraps() {
        let mut popup = SortPopup::new(SortSpec::default());
        popup.cursor = 0;

        popup.cursor_up();
        assert_eq!(popup.cursor, SortPopup::ROWS - 1);
        popup.cursor_down();
        assert_eq!(popup.cursor, 0);
    }

    #[test]
    fn sort_popup_activate_selects_keys_and_flips_toggles() {
        let mut popup = SortPopup::new(SortSpec::default());

        popup.cursor = 4;
        popup.activate_cursor();
        assert_eq!(popup.draft.key, SortKey::Time);

        popup.cursor = 6;
        popup.activate_cursor();
        assert!(popup.draft.descending);
        popup.activate_cursor();
        assert!(!popup.draft.descending);
    }

    #[test]
    fn sort_popup_letters() {
        let mut popup = SortPopup::new(SortSpec::default());

        assert_eq!(SortPopup::key_for_letter('X'), Some(SortKey::Natural));
        assert_eq!(SortPopup::key_for_letter('D'), None);
        assert!(popup.toggle_for_letter('C'));
        assert!(!popup.draft.ignore_case);
        assert!(popup.toggle_for_letter('G'));
        assert!(!popup.draft.dirs_first);
        assert!(!popup.toggle_for_letter('Q'));
    }

    #[test]
    fn palette_filters_by_substring_ignoring_case() {
        let mut popup = PalettePopup::new(Context::FileList);
        assert_eq!(popup.matches.len(), Command::all().count());

        popup.set_query("MARK_R".to_owned());

        assert_eq!(
            popup.matches,
            [Command::MarkRangeAbove, Command::MarkRangeBelow]
        );
        assert_eq!(popup.selected(), Some(Command::MarkRangeAbove));
    }

    #[test]
    fn palette_shows_only_commands_available_in_the_context() {
        let popup = PalettePopup::new(Context::Image);

        assert_eq!(popup.matches.len(), 16);
        assert!(!popup.matches.contains(&Command::Copy));
    }

    #[test]
    fn palette_cursor_wraps_and_handles_no_matches() {
        let mut popup = PalettePopup::new(Context::FileList);
        popup.set_query("jump_".to_owned());
        assert_eq!(popup.matches.len(), 36);

        popup.cursor_up();
        assert_eq!(popup.cursor, 35);
        popup.cursor_down();
        assert_eq!(popup.cursor, 0);

        popup.set_query("nothing matches this".to_owned());
        popup.cursor_down();
        assert_eq!(popup.selected(), None);
    }
}
