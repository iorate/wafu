use std::ffi::OsString;
use std::path::PathBuf;

use crate::command::{Command, Context};
use crate::entry::Entry;
use crate::error::{AppError, IoError};
use crate::key::{Key, KeyInput};
use crate::keymap::KeyMap;
use crate::location::Location;
use crate::pane::{Pane, RequestId};
use crate::view::{PaneView, View};

/// An event delivered to the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// The application has started with the given initial pane locations.
    Start {
        left: PathBuf,
        right: PathBuf,
    },
    Key(KeyInput),
    /// The number of rows that fit in a pane's list.
    ViewportRows {
        side: Side,
        rows: usize,
    },
    Io(IoEvent),
}

/// A request from the application to the outside world.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    ReadDir {
        side: Side,
        location: Location,
        request: RequestId,
    },
    CancelRead {
        request: RequestId,
    },
    SetTitle(String),
}

/// The result of an effect, reported by the I/O layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IoEvent {
    DirLoaded {
        side: Side,
        request: RequestId,
        result: Result<Vec<Entry>, IoError>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// The whole application state.
#[derive(Debug)]
pub struct App {
    panes: [Pane; 2],
    active: Side,
    keymap: KeyMap,
    log: Vec<String>,
    next_request: u64,
}

impl Side {
    #[must_use]
    pub fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }

    fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

impl App {
    #[must_use]
    pub fn new() -> Self {
        Self {
            panes: [
                Pane::new(Location::new(PathBuf::new())),
                Pane::new(Location::new(PathBuf::new())),
            ],
            active: Side::Left,
            keymap: KeyMap::default(),
            log: Vec::new(),
            next_request: 1,
        }
    }

    /// Applies a message and returns the effects it produced.
    pub fn update(&mut self, msg: Msg) -> Vec<Effect> {
        let title_before = self.title();
        let mut effects = Vec::new();
        match msg {
            Msg::Start { left, right } => {
                self.panes = [
                    Pane::new(Location::new(PathBuf::new())),
                    Pane::new(Location::new(PathBuf::new())),
                ];
                self.navigate(Side::Left, Location::new(left), None, &mut effects);
                self.navigate(Side::Right, Location::new(right), None, &mut effects);
                effects.push(Effect::SetTitle(self.title()));
                return effects;
            }
            Msg::Key(input) => {
                if let Some(key) = Key::from_input(&input)
                    && let Some(command) = self.keymap.lookup(Context::FileList, &key)
                {
                    self.run_command(command, &mut effects);
                }
            }
            Msg::ViewportRows { side, rows } => self.panes[side.index()].set_viewport_rows(rows),
            Msg::Io(IoEvent::DirLoaded {
                side,
                request,
                result,
            }) => self.dir_loaded(side, request, result),
        }
        let title = self.title();
        if title != title_before {
            effects.push(Effect::SetTitle(title));
        }
        effects
    }

    #[must_use]
    pub fn view(&self) -> View {
        View {
            title: self.title(),
            left: PaneView::from_pane(&self.panes[0], self.active == Side::Left),
            right: PaneView::from_pane(&self.panes[1], self.active == Side::Right),
            log: self.log.clone(),
        }
    }

    #[must_use]
    pub fn pane(&self, side: Side) -> &Pane {
        &self.panes[side.index()]
    }

    #[must_use]
    pub fn active(&self) -> Side {
        self.active
    }

    fn run_command(&mut self, command: Command, effects: &mut Vec<Effect>) {
        let side = self.active;
        let pane = &mut self.panes[side.index()];
        match command {
            Command::Open => match pane.cursor_entry() {
                None => self.go_parent(effects),
                Some(entry) if entry.kind.is_dir_like() => {
                    let location = Location::new(pane.location.base.join(&entry.name));
                    self.navigate(side, location, None, effects);
                }
                Some(_) => {}
            },
            Command::GoParent => self.go_parent(effects),
            Command::FocusLeft => self.active = Side::Left,
            Command::FocusRight => self.active = Side::Right,
            Command::SwitchPane => self.active = side.other(),
            Command::CursorUp => pane.cursor_up(),
            Command::CursorDown => pane.cursor_down(),
            Command::PageUp => pane.page_up(),
            Command::PageDown => pane.page_down(),
            Command::CursorTop => pane.cursor_top(),
            Command::CursorBottom => pane.cursor_bottom(),
            _ => {}
        }
    }

    fn go_parent(&mut self, effects: &mut Vec<Effect>) {
        let side = self.active;
        let pane = &self.panes[side.index()];
        if let Some(parent) = pane.location.parent() {
            let came_from = pane.location.base.file_name().map(OsString::from);
            self.navigate(side, parent, came_from, effects);
        }
    }

    fn navigate(
        &mut self,
        side: Side,
        location: Location,
        pending_cursor: Option<OsString>,
        effects: &mut Vec<Effect>,
    ) {
        let request = RequestId(self.next_request);
        self.next_request += 1;
        let pane = &mut self.panes[side.index()];
        if let Some(old) = pane.loading.take() {
            effects.push(Effect::CancelRead { request: old });
        }
        pane.set_location(location.clone(), pending_cursor);
        pane.loading = Some(request);
        effects.push(Effect::ReadDir {
            side,
            location,
            request,
        });
    }

    fn dir_loaded(&mut self, side: Side, request: RequestId, result: Result<Vec<Entry>, IoError>) {
        let pane = &mut self.panes[side.index()];
        if pane.loading != Some(request) {
            return;
        }
        pane.loading = None;
        match result {
            Ok(entries) => pane.set_entries(entries),
            Err(source) => {
                let error = AppError::ReadDir {
                    path: pane.location.base.clone(),
                    source,
                };
                self.log.push(error.to_string());
            }
        }
    }

    fn title(&self) -> String {
        let path = self.panes[self.active.index()].location.to_string();
        if path.is_empty() {
            "wafu".to_owned()
        } else {
            format!("wafu - {path}")
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entry::{EntryKind, TargetKind};
    use crate::test_support::{Harness, dir, file};

    #[test]
    fn start_reads_both_panes_and_sets_title() {
        let mut harness = Harness::start("/home/user", "/tmp");

        let effects = harness.take_effects();

        assert_eq!(
            effects,
            vec![
                Effect::ReadDir {
                    side: Side::Left,
                    location: Location::new(PathBuf::from("/home/user")),
                    request: RequestId(1),
                },
                Effect::ReadDir {
                    side: Side::Right,
                    location: Location::new(PathBuf::from("/tmp")),
                    request: RequestId(2),
                },
                Effect::SetTitle("wafu - /home/user".to_owned()),
            ]
        );
        assert_eq!(harness.view().title, "wafu - /home/user");
    }

    #[test]
    fn loaded_entries_appear_after_the_parent_row() {
        let mut harness = Harness::start("/home/user", "/tmp");

        harness.load(Side::Left, vec![file("b.txt"), dir("a")]);

        assert_eq!(harness.names(Side::Left), ["..", "a", "b.txt"]);
        assert_eq!(harness.names(Side::Right), [".."]);
        assert_eq!(harness.view().left.cursor, Some(0));
        assert_eq!(
            harness.view().left.info,
            "2 items | sort: name | hidden: off"
        );
    }

    #[test]
    fn extension_column_includes_the_dot() {
        let mut harness = Harness::start("/home/user", "/tmp");

        harness.load(
            Side::Left,
            vec![file("a.tar.gz"), file("noext"), dir("conf.d")],
        );

        let view = harness.view();
        let columns: Vec<(&str, &str)> = view
            .left
            .rows
            .iter()
            .map(|row| (row.name.as_str(), row.extension.as_str()))
            .collect();
        assert_eq!(
            columns,
            [("..", ""), ("conf.d", ""), ("a.tar", ".gz"), ("noext", "")]
        );
    }

    #[test]
    fn size_and_time_columns_follow_symlink_targets() {
        let mut harness = Harness::start("/home/user", "/tmp");
        let to_dir = Entry {
            kind: EntryKind::Symlink {
                target: Some(TargetKind::Dir),
            },
            ..file("to_dir")
        };
        let to_file = Entry {
            kind: EntryKind::Symlink {
                target: Some(TargetKind::File),
            },
            size: 42,
            ..file("to_file")
        };
        let broken = Entry {
            kind: EntryKind::Symlink { target: None },
            ..file("broken")
        };

        harness.load(Side::Left, vec![to_dir, to_file, broken]);

        let view = harness.view();
        let columns: Vec<(&str, &str, &str)> = view
            .left
            .rows
            .iter()
            .map(|row| (row.name.as_str(), row.size.as_str(), row.modified.as_str()))
            .collect();
        assert_eq!(
            columns,
            [
                ("..", "", ""),
                ("to_dir", "<DIR>", ""),
                ("broken", "?", "?"),
                ("to_file", "42", ""),
            ]
        );
    }

    #[test]
    fn root_has_no_parent_row() {
        let mut harness = Harness::start("/", "/tmp");

        harness.load(Side::Left, vec![file("a")]);

        assert_eq!(harness.names(Side::Left), ["a"]);
    }

    #[test]
    fn stale_reply_is_discarded() {
        let mut harness = Harness::start("/home/user", "/tmp");
        let stale = harness.latest_request(Side::Left);
        harness.press("Enter");

        harness.send(Msg::Io(IoEvent::DirLoaded {
            side: Side::Left,
            request: stale,
            result: Ok(vec![file("stale")]),
        }));

        assert_eq!(harness.names(Side::Left), [".."]);
    }

    #[test]
    fn open_directory_navigates_and_cancels_pending_read() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.load(Side::Left, vec![dir("docs")]);
        harness.press("Down");
        harness.take_effects();

        harness.press("Enter");
        harness.press("BS");

        // `join` uses the platform separator, so the expected path is built the same way.
        let docs = PathBuf::from("/home/user").join("docs");
        assert_eq!(
            harness.take_effects(),
            vec![
                Effect::ReadDir {
                    side: Side::Left,
                    location: Location::new(docs.clone()),
                    request: RequestId(3),
                },
                Effect::SetTitle(format!("wafu - {}", docs.display())),
                Effect::CancelRead {
                    request: RequestId(3)
                },
                Effect::ReadDir {
                    side: Side::Left,
                    location: Location::new(PathBuf::from("/home/user")),
                    request: RequestId(4),
                },
                Effect::SetTitle("wafu - /home/user".to_owned()),
            ]
        );
        assert_eq!(harness.view().left.path, "/home/user");
    }

    #[test]
    fn open_on_parent_row_goes_to_parent() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.load(Side::Left, vec![file("a")]);

        harness.press("Enter");

        assert_eq!(harness.view().left.path, "/home");
    }

    #[test]
    fn open_on_file_does_nothing() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.load(Side::Left, vec![file("a")]);
        harness.press("Down");
        harness.take_effects();

        harness.press("Enter");

        assert_eq!(harness.take_effects(), vec![]);
        assert_eq!(harness.view().left.path, "/home/user");
    }

    #[test]
    fn go_parent_puts_cursor_on_the_directory_we_came_from() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.load(Side::Left, vec![]);

        harness.press("BS");
        harness.load(Side::Left, vec![dir("other"), dir("user")]);

        assert_eq!(harness.view().left.path, "/home");
        assert_eq!(harness.cursor_name(Side::Left), "user");
    }

    #[test]
    fn go_parent_at_root_does_nothing() {
        let mut harness = Harness::start("/", "/tmp");
        harness.load(Side::Left, vec![]);
        harness.take_effects();

        harness.press("BS");

        assert_eq!(harness.take_effects(), vec![]);
    }

    #[test]
    fn focus_left_when_already_left_does_nothing() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.take_effects();

        harness.press("Left");

        assert_eq!(harness.take_effects(), vec![]);
        assert_eq!(harness.app().active(), Side::Left);
    }

    #[test]
    fn focus_and_switch_change_the_active_pane_and_title() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.take_effects();

        harness.press("Right");
        assert_eq!(
            harness.take_effects(),
            vec![Effect::SetTitle("wafu - /tmp".to_owned())]
        );
        assert!(harness.view().right.active);

        harness.press("Tab");
        assert_eq!(
            harness.take_effects(),
            vec![Effect::SetTitle("wafu - /home/user".to_owned())]
        );
        assert!(harness.view().left.active);
    }

    #[test]
    fn cursor_is_shown_only_in_the_active_pane() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.load(Side::Right, vec![file("a")]);

        let view = harness.view();

        assert_eq!(view.left.cursor, Some(0));
        assert_eq!(view.right.cursor, None);
    }

    #[test]
    fn cursor_keys_move_the_cursor_of_the_active_pane() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.load(Side::Left, vec![file("a"), file("b"), file("c")]);
        harness.viewport(Side::Left, 2);

        harness.press("Down");
        assert_eq!(harness.cursor_name(Side::Left), "a");
        harness.press("End");
        assert_eq!(harness.cursor_name(Side::Left), "c");
        harness.press("Down");
        assert_eq!(harness.cursor_name(Side::Left), "..");
        harness.press("Up");
        assert_eq!(harness.cursor_name(Side::Left), "c");
        harness.press("PageUp");
        assert_eq!(harness.cursor_name(Side::Left), "a");
        harness.press("Home");
        assert_eq!(harness.cursor_name(Side::Left), "..");
        harness.press("PageDown");
        assert_eq!(harness.cursor_name(Side::Left), "b");
        assert_eq!(harness.view().left.scroll_top, 2);
    }

    #[test]
    fn navigating_empties_the_listing_until_the_new_one_arrives() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.load(Side::Left, vec![dir("docs")]);
        harness.press("Down");

        harness.press("Enter");

        let docs = PathBuf::from("/home/user").join("docs");
        assert_eq!(harness.view().left.path, docs.display().to_string());
        assert_eq!(harness.names(Side::Left), [".."]);
        assert_eq!(harness.view().left.cursor, Some(0));
    }

    #[test]
    fn read_error_is_logged_and_the_pane_stays_at_the_new_location() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.load(Side::Left, vec![file("a")]);
        harness.press("BS");

        harness.fail_load(Side::Left, "permission denied");

        let view = harness.view();
        assert_eq!(view.log, vec!["cannot read /home: permission denied"]);
        assert_eq!(view.left.path, "/home");
        assert_eq!(harness.names(Side::Left), [".."]);
        assert_eq!(harness.app().pane(Side::Left).loading, None);
    }

    #[test]
    fn unbound_key_does_nothing() {
        let mut harness = Harness::start("/home/user", "/tmp");
        harness.take_effects();

        harness.press("F1");

        assert_eq!(harness.take_effects(), vec![]);
    }
}
