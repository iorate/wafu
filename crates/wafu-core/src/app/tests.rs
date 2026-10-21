use std::path::{Path, PathBuf};

use super::*;
use crate::config::Theme;
use crate::entry::{EntryKind, TargetKind};
use crate::external::CommandLine;
use crate::listing::{SortKey, SortSpec};
use crate::pane::InFlight;
use crate::state::{PaneState, StateFile};
use crate::test_support::{CONFIG_PATH, HOME, Harness, dir, file, start_msg_with};
use crate::view::{Ellipsis, ItemMark, RowKind};

mod archives;
mod operations;

#[test]
fn start_resolves_both_panes_then_watches_and_reads_them() {
    let mut harness = Harness::start_unresolved(Os::Linux, "/home/user", "/tmp");

    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::WatchConfig(PathBuf::from(CONFIG_PATH)),
            Effect::Resolve {
                side: Side::Left,
                path: PathBuf::from("/home/user"),
                request: RequestId(1),
            },
            Effect::Resolve {
                side: Side::Right,
                path: PathBuf::from("/tmp"),
                request: RequestId(2),
            },
            Effect::SetTitle("wafu".to_owned()),
        ]
    );
    assert_eq!(harness.view().title, "wafu");

    harness.resolve(Side::Left, "/home/user", None);

    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::Watch {
                side: Side::Left,
                path: PathBuf::from("/home/user"),
            },
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(PathBuf::from("/home/user")),
                request: RequestId(3),
            },
        ]
    );
    assert_eq!(harness.view().left.path, "");

    harness.load(Side::Left, vec![file("a")]);

    assert_eq!(
        harness.take_effects(),
        vec![Effect::SetTitle("wafu - /home/user".to_owned())]
    );
    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.names(Side::Left), ["..", "a"]);
    assert_eq!(harness.history(Side::Left), ["/home/user"]);
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
        "2 items | 0 marked (0 bytes) | sort: name | hidden: off"
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
    let columns: Vec<(&str, &str, &str)> = view
        .left
        .rows
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                row.extension.as_str(),
                row.marker.as_str(),
            )
        })
        .collect();
    assert_eq!(
        columns,
        [
            ("..", "", ""),
            ("conf.d", "", ""),
            ("a.tar", ".gz", ""),
            ("noext", "", ""),
        ]
    );
}

#[test]
fn long_names_are_split_for_middle_ellipsis() {
    let mut harness = Harness::start("/home/user", "/tmp");

    harness.load(Side::Left, vec![file("a_rather_long_name.txt")]);

    let view = harness.view();
    assert_eq!(view.style.ellipsis, Ellipsis::End);
    assert_eq!(view.left.rows[1].name, "a_rather_l");
    assert_eq!(view.left.rows[1].name_tail, "ong_name");
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
    let columns: Vec<(&str, &str, &str, &str)> = view
        .left
        .rows
        .iter()
        .map(|row| {
            (
                row.name.as_str(),
                row.marker.as_str(),
                row.size.as_str(),
                row.modified.as_str(),
            )
        })
        .collect();
    assert_eq!(
        columns,
        [
            ("..", "", "", ""),
            ("to_dir", "@", "<DIR>", ""),
            ("broken", "@", "?", "?"),
            ("to_file", "@", "42", ""),
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
    assert!(harness.view().popup.is_some_and(|popup| popup.progress));
}

#[test]
fn open_directory_navigates_and_cancels_pending_read() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![dir("docs")]);
    harness.press("Down");
    harness.take_effects();

    harness.press("Enter");
    // The move is modal, so a second move can only come from the outside.
    harness.press("BS");
    harness.press("Esc");

    // `join` uses the platform separator, so the expected path is built the same way.
    let docs = PathBuf::from("/home/user").join("docs");
    let request = harness.latest_request(Side::Left);
    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::Watch {
                side: Side::Left,
                path: docs.clone(),
            },
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(docs.clone()),
                request,
            },
            Effect::CancelRead { request },
            Effect::Watch {
                side: Side::Left,
                path: PathBuf::from("/home/user"),
            },
        ]
    );
    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.cursor_name(Side::Left), "docs");
    assert!(harness.view().popup.is_none());
}

#[test]
fn open_on_parent_row_goes_to_parent() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);

    harness.press("Enter");
    harness.load(Side::Left, vec![]);

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
fn navigating_keeps_the_old_listing_until_the_new_one_arrives() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![dir("docs")]);
    harness.press("Down");
    harness.press("Space");

    harness.press("Enter");

    let docs = PathBuf::from("/home/user").join("docs");
    let view = harness.view();
    assert_eq!(view.left.path, "/home/user");
    assert_eq!(view.title, "wafu - /home/user");
    assert_eq!(harness.names(Side::Left), ["..", "docs"]);
    assert_eq!(harness.marks(Side::Left), ["docs"]);
    let popup = view.popup.unwrap();
    assert_eq!(popup.title, "Loading");
    assert_eq!(popup.message, Some(format!("Reading {}", docs.display())));
    assert!(popup.progress);

    harness.load(Side::Left, vec![file("x")]);

    let view = harness.view();
    assert_eq!(view.left.path, docs.display().to_string());
    assert_eq!(harness.names(Side::Left), ["..", "x"]);
    assert_eq!(view.left.cursor, Some(0));
    assert!(view.popup.is_none());
    assert_eq!(
        harness.history(Side::Left),
        ["/home/user".to_owned(), docs.display().to_string()]
    );
}

#[test]
fn keys_and_mouse_are_dropped_while_a_move_is_in_progress() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("BS");
    harness.take_effects();

    harness.press("Down");
    harness.press("Space");
    harness.click(Side::Right, 0);

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.app().active(), Side::Left);
}

#[test]
fn escape_cancels_a_move_and_stays_put() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.press("BS");
    let pending = harness.latest_request(Side::Left);
    harness.take_effects();

    harness.press("Esc");

    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::CancelRead { request: pending },
            Effect::Watch {
                side: Side::Left,
                path: PathBuf::from("/home/user"),
            },
        ]
    );
    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.names(Side::Left), ["..", "a"]);
    assert!(harness.view().popup.is_none());

    // The listing that arrives late belongs to the cancelled move.
    harness.send(Msg::Io(IoEvent::DirLoaded {
        side: Side::Left,
        request: pending,
        result: Ok(vec![dir("user")]),
    }));
    assert_eq!(harness.view().left.path, "/home/user");
}

#[test]
fn read_error_is_logged_and_the_pane_stays_where_it_was() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.press("BS");
    harness.take_effects();

    harness.fail_load(Side::Left, "permission denied");

    assert_eq!(harness.log(), ["cannot read /home: permission denied"]);
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Watch {
            side: Side::Left,
            path: PathBuf::from("/home/user"),
        }]
    );
    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.names(Side::Left), ["..", "a"]);
    assert!(harness.app().pane(Side::Left).loading().is_none());
    assert_eq!(harness.history(Side::Left), ["/home/user"]);
}

#[test]
fn unbound_key_does_nothing() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("F1");

    assert_eq!(harness.take_effects(), vec![]);
}

// Marks

#[test]
fn space_marks_and_moves_down_and_the_info_line_counts_marks() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(
        Side::Left,
        vec![
            Entry {
                size: 10,
                ..file("a")
            },
            Entry {
                size: 5,
                ..file("b")
            },
            Entry {
                size: 4096,
                ..dir("d")
            },
        ],
    );

    harness.press("Down");
    harness.press("Space");
    harness.press("Space");

    assert_eq!(harness.marks(Side::Left), ["d", "a"]);
    assert_eq!(harness.cursor_name(Side::Left), "b");
    assert_eq!(
        harness.view().left.info,
        "3 items | 2 marked (10 bytes) | sort: name | hidden: off"
    );
    assert_eq!(
        harness.view().left.marks.as_ref(),
        [false, true, true, false]
    );
}

#[test]
fn shift_space_marks_and_moves_up() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("End");

    harness.press("Shift+Space");

    assert_eq!(harness.marks(Side::Left), ["b"]);
    assert_eq!(harness.cursor_name(Side::Left), "a");
}

#[test]
fn ctrl_space_marks_the_range_from_the_mark_above() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b"), file("c"), file("d")]);
    harness.press("Down");
    harness.press("Space");
    harness.press("Down");
    harness.press("Down");

    harness.press("Ctrl+Space");

    assert_eq!(harness.marks(Side::Left), ["a", "b", "c", "d"]);
}

#[test]
fn ctrl_shift_space_marks_the_range_to_the_mark_below() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b"), file("c"), file("d")]);
    harness.press("End");
    harness.press("Shift+Space");
    harness.press("Home");
    harness.press("Down");

    harness.press("Ctrl+Shift+Space");

    assert_eq!(harness.marks(Side::Left), ["a", "b", "c", "d"]);
}

#[test]
fn invert_marks_with_and_without_directories() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![dir("d"), file("a"), file("b")]);

    harness.press("A");
    assert_eq!(harness.marks(Side::Left), ["a", "b"]);

    harness.press("Shift+A");
    assert_eq!(harness.marks(Side::Left), ["d"]);
}

#[test]
fn mark_glob_adds_matching_files() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(
        Side::Left,
        vec![dir("d.png"), file("a.png"), file("b.txt"), file("c.PNG")],
    );
    harness.press("End");
    harness.press("Space");

    harness.press("@");
    assert_eq!(harness.view().popup.unwrap().title, "Mark");
    harness.edit("*.png");
    harness.press("Enter");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.marks(Side::Left), ["a.png", "c.PNG"]);
}

#[test]
fn mark_glob_with_empty_text_does_nothing() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);

    harness.press("@");
    harness.press("Enter");

    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn next_mark_cycles_through_marks() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b"), file("c")]);
    harness.press("Down");
    harness.press("Space");
    harness.press("Down");
    harness.press("Space");
    harness.press("Home");

    harness.press("Ctrl+:");
    assert_eq!(harness.cursor_name(Side::Left), "a");
    harness.press("Ctrl+:");
    assert_eq!(harness.cursor_name(Side::Left), "c");
    harness.press("Ctrl+:");
    assert_eq!(harness.cursor_name(Side::Left), "a");
    harness.press("Ctrl+*");
    assert_eq!(harness.cursor_name(Side::Left), "c");
    harness.press("Ctrl+*");
    assert_eq!(harness.cursor_name(Side::Left), "a");
}

#[test]
fn marks_are_dropped_on_navigation_and_do_not_come_back() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![dir("docs"), file("a")]);
    harness.press("End");
    harness.press("Space");
    harness.press("Home");
    harness.press("Down");

    harness.press("Enter");
    harness.load(Side::Left, vec![file("x")]);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());

    harness.press("BS");
    harness.load(Side::Left, vec![dir("docs"), file("a")]);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
}

#[test]
fn marks_survive_sorting_and_reloading() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("Down");
    harness.press("Space");

    harness.press("S");
    harness.press("D");
    harness.press("Enter");
    assert_eq!(harness.names(Side::Left), ["..", "b", "a"]);
    assert_eq!(harness.marks(Side::Left), ["a"]);

    harness.press("Y");
    harness.load(Side::Left, vec![file("a"), file("b"), file("c")]);
    assert_eq!(harness.marks(Side::Left), ["a"]);
}

// Display settings

#[test]
fn toggle_hidden_shows_hidden_entries_and_clears_marks() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(
        Side::Left,
        vec![
            file("a"),
            Entry {
                hidden: true,
                ..file(".b")
            },
        ],
    );
    harness.press("Down");
    harness.press("Space");

    harness.press(".");

    assert_eq!(harness.names(Side::Left), ["..", ".b", "a"]);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.cursor_name(Side::Left), "a");
    assert!(harness.view().left.info.ends_with("hidden: on"));
}

#[test]
fn filter_popup_applies_a_glob_and_shows_it_in_the_info_line() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![dir("d.txt"), file("a.png"), file("b.txt")]);

    harness.press(":");
    let popup = harness.view().popup.unwrap();
    assert_eq!(
        (
            popup.title.as_str(),
            popup.fields.first().map(|field| field.text.as_str())
        ),
        ("Filter", Some(""))
    );
    harness.edit("*.png");
    harness.press("Enter");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.names(Side::Left), ["..", "d.txt", "a.png"]);
    assert_eq!(
        harness.view().left.info,
        "2 items | 0 marked (0 bytes) | filter: *.png | sort: name | hidden: off"
    );
}

#[test]
fn filter_popup_starts_with_the_current_pattern() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a.png")]);
    harness.press(":");
    harness.edit("*.png");
    harness.press("Enter");

    harness.press(":");

    assert_eq!(
        harness
            .view()
            .popup
            .unwrap()
            .fields
            .first()
            .map(|field| field.text.as_str()),
        Some("*.png")
    );
}

#[test]
fn filter_change_clears_marks_but_the_same_pattern_keeps_them() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(
        Side::Left,
        vec![file("a.png"), file("b.png"), file("c.txt")],
    );
    harness.press(":");
    harness.edit("*.png");
    harness.press("Enter");
    harness.press("Down");
    harness.press("Space");

    harness.press(":");
    harness.press("Enter");
    assert_eq!(harness.marks(Side::Left), ["a.png"]);

    harness.press(":");
    harness.edit("*.txt");
    harness.press("Enter");
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.names(Side::Left), ["..", "c.txt"]);
}

#[test]
fn clear_filter_restores_the_listing_and_clears_marks() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a.png"), file("b.txt")]);
    harness.press(":");
    harness.edit("*.png");
    harness.press("Enter");
    harness.press("Down");
    harness.press("Space");

    harness.press("*");

    assert_eq!(harness.names(Side::Left), ["..", "a.png", "b.txt"]);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.app().pane(Side::Left).filter(), None);
}

#[test]
fn empty_filter_text_clears_the_filter() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a.png"), file("b.txt")]);
    harness.press(":");
    harness.edit("*.png");
    harness.press("Enter");

    harness.press(":");
    harness.edit("");
    harness.press("Enter");

    assert_eq!(harness.names(Side::Left), ["..", "a.png", "b.txt"]);
}

#[test]
fn invalid_pattern_is_logged_and_leaves_the_filter_alone() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a.png")]);

    harness.press(":");
    harness.edit("[");
    harness.press("Enter");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.log().len(), 1);
    assert!(harness.log()[0].starts_with("invalid pattern [: "));
    assert_eq!(harness.app().pane(Side::Left).filter(), None);
}

#[test]
fn filter_is_dropped_on_navigation() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![dir("docs"), file("a.png")]);
    harness.press(":");
    harness.edit("*.png");
    harness.press("Enter");

    harness.press("Down");
    harness.press("Enter");
    harness.load(Side::Left, vec![]);

    assert_eq!(harness.app().pane(Side::Left).filter(), None);
}

#[test]
fn escape_closes_the_filter_popup_without_applying() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a.png"), file("b.txt")]);

    harness.press(":");
    harness.edit("*.png");
    harness.press("Esc");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.names(Side::Left), ["..", "a.png", "b.txt"]);
}

// Sort popup

#[test]
fn sort_popup_shows_the_draft_and_letters_confirm_immediately() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("b.txt"), file("a.zip")]);

    harness.press("S");
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Sort");
    assert_eq!(popup.fields, Vec::new());
    assert_eq!(popup.items.len(), 9);
    assert_eq!(popup.items[1].label, "Name");
    assert_eq!(popup.items[1].mark, ItemMark::Radio(true));
    assert_eq!(popup.cursor, Some(1));

    harness.press("E");

    assert_eq!(harness.popup(), None);
    assert_eq!(
        harness.app().pane(Side::Left).sort().key,
        SortKey::Extension
    );
    assert_eq!(harness.names(Side::Left), ["..", "b.txt", "a.zip"]);
}

#[test]
fn sort_popup_toggles_stay_open_until_enter() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);

    harness.press("S");
    harness.press("D");
    assert!(harness.popup().is_some());
    assert_eq!(
        harness.view().popup.unwrap().items[6].mark,
        ItemMark::Check(true)
    );
    assert_eq!(harness.names(Side::Left), ["..", "a", "b"]);

    harness.press("Enter");
    assert_eq!(harness.popup(), None);
    assert_eq!(harness.names(Side::Left), ["..", "b", "a"]);
}

#[test]
fn sort_popup_cursor_and_space_edit_the_draft() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), dir("d")]);

    harness.press("S");
    harness.press("Up");
    harness.press("Up");
    assert_eq!(harness.view().popup.unwrap().cursor, Some(8));
    harness.press("Up");
    harness.press("Space");
    assert_eq!(
        harness.view().popup.unwrap().items[7].mark,
        ItemMark::Check(false)
    );
    harness.press("Enter");

    assert_eq!(harness.names(Side::Left), ["..", "a", "d"]);
}

#[test]
fn sort_popup_escape_discards_and_keys_do_not_reach_the_list() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("Down");

    harness.press("S");
    harness.press("D");
    harness.press("Space");
    harness.press("Esc");

    assert_eq!(harness.popup(), None);
    assert!(!harness.app().pane(Side::Left).sort().descending);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.cursor_name(Side::Left), "a");
}

#[test]
fn sort_change_keeps_the_cursor_on_the_same_entry() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b"), file("c")]);
    harness.press("Down");

    harness.press("S");
    harness.press("D");
    harness.press("Enter");

    assert_eq!(harness.cursor_name(Side::Left), "a");
    assert_eq!(harness.view().left.cursor, Some(3));
}

// Search

#[test]
fn search_moves_the_cursor_as_the_text_grows_and_refuses_misses() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(
        Side::Left,
        vec![file("apple"), file("banana"), file("berry")],
    );

    harness.press("F");
    assert_eq!(harness.view().popup.unwrap().title, "Search");
    harness.edit("b");
    assert_eq!(harness.cursor_name(Side::Left), "banana");
    harness.edit("be");
    assert_eq!(harness.cursor_name(Side::Left), "berry");

    harness.edit("bez");
    assert_eq!(
        harness
            .view()
            .popup
            .unwrap()
            .fields
            .first()
            .map(|field| field.text.as_str()),
        Some("be")
    );
    assert_eq!(harness.cursor_name(Side::Left), "berry");
}

#[test]
fn search_arrows_move_between_matches_and_enter_keeps_the_cursor() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a1"), file("a2"), file("b")]);

    harness.press("F");
    harness.edit("a");
    harness.press("Down");
    assert_eq!(harness.cursor_name(Side::Left), "a2");
    harness.press("Down");
    assert_eq!(harness.cursor_name(Side::Left), "a1");
    harness.press("Up");
    assert_eq!(harness.cursor_name(Side::Left), "a2");
    harness.press("Enter");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.cursor_name(Side::Left), "a2");
}

#[test]
fn search_escape_restores_the_cursor_and_scroll() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b"), file("c"), file("d")]);
    harness.viewport(Side::Left, 2);
    harness.press("Down");

    harness.press("F");
    harness.edit("d");
    assert_eq!(harness.view().left.scroll_top, 3);
    harness.press("Esc");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.cursor_name(Side::Left), "a");
    assert_eq!(harness.view().left.scroll_top, 0);
}

#[test]
fn search_emptied_returns_to_the_origin_cursor_and_scroll() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b"), file("c"), file("d")]);
    harness.viewport(Side::Left, 2);
    harness.press("Down");

    harness.press("F");
    harness.edit("d");
    assert_eq!(harness.view().left.scroll_top, 3);
    harness.edit("");

    assert_eq!(harness.cursor_name(Side::Left), "a");
    assert_eq!(harness.view().left.scroll_top, 0);
}

#[test]
fn jump_moves_to_the_next_entry_starting_with_the_letter() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("alpha"), file("beta"), file("Bravo")]);

    harness.press("Ctrl+B");
    assert_eq!(harness.cursor_name(Side::Left), "beta");
    harness.press("Ctrl+B");
    assert_eq!(harness.cursor_name(Side::Left), "Bravo");
    harness.press("Ctrl+B");
    assert_eq!(harness.cursor_name(Side::Left), "beta");
}

// Palette

#[test]
fn palette_filters_commands_and_runs_the_selected_one() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.press("Down");
    harness.press("Space");

    harness.press("B");
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Palette");
    assert_eq!(popup.items.len(), Command::all().count());
    assert_eq!(popup.cursor, Some(0));

    harness.edit("clear_m");
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.items.len(), 1);
    assert_eq!(popup.items[0].label, "clear_marks");
    assert_eq!(popup.items[0].detail, "");
    harness.press("Enter");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
}

#[test]
fn palette_shows_bound_keys_and_wraps_the_cursor() {
    let mut harness = Harness::start("/home/user", "/tmp");

    harness.press("B");
    harness.edit("mark_range");
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.items[0].detail, "Ctrl+Space");
    assert_eq!(popup.items[1].detail, "Ctrl+Shift+Space");
    harness.press("Up");
    assert_eq!(harness.view().popup.unwrap().cursor, Some(1));
    harness.press("Down");
    assert_eq!(harness.view().popup.unwrap().cursor, Some(0));
}

#[test]
fn palette_with_no_match_closes_on_enter_and_escape_closes_too() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("B");
    harness.edit("no such command");
    assert_eq!(harness.view().popup.unwrap().cursor, None);
    harness.press("Enter");
    assert_eq!(harness.popup(), None);

    harness.press("B");
    harness.press("Esc");
    assert_eq!(harness.popup(), None);
    assert_eq!(harness.take_effects(), vec![]);
}

// Log and layout

#[test]
fn log_scroll_range_follows_the_log_height_from_the_start() {
    let mut harness = Harness::start("/a/b/c/d/e/f/g/h", "/tmp");
    for _ in 0..7 {
        harness.press("BS");
        harness.fail_load(Side::Left, "boom");
    }
    assert_eq!(harness.view().log_rows, 6);
    assert_eq!(harness.view().log.scroll_top, 1);

    harness.press("Alt+Up");
    assert_eq!(harness.view().log.scroll_top, 0);
    harness.press("Alt+Down");
    harness.press("Alt+Down");
    assert_eq!(harness.view().log.scroll_top, 2);
    harness.press("Shift+Up");
    assert_eq!(harness.view().log.scroll_top, 1);
}

#[test]
fn log_keys_scroll_and_clear() {
    let mut harness = Harness::start("/a/b/c/d/e", "/tmp");
    for _ in 0..4 {
        harness.press("Alt+Down");
    }
    for _ in 0..4 {
        harness.press("BS");
        harness.fail_load(Side::Left, "boom");
    }
    assert_eq!(harness.log().len(), 4);
    assert_eq!(harness.view().log.scroll_top, 2);

    harness.press("Shift+Up");
    assert_eq!(harness.view().log.scroll_top, 1);
    harness.press("Shift+PageUp");
    assert_eq!(harness.view().log.scroll_top, 0);
    harness.press("Shift+PageDown");
    assert_eq!(harness.view().log.scroll_top, 2);
    harness.press("Shift+Up");
    harness.press("Shift+Down");
    assert_eq!(harness.view().log.scroll_top, 2);

    harness.send(Msg::Key(KeyInput {
        text: "clear_log".to_owned(),
        ctrl: false,
        shift: false,
        alt: false,
    }));
    harness.press("B");
    harness.edit("clear_log");
    harness.press("Enter");
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn layout_keys_change_the_split_and_log_height_within_bounds() {
    let mut harness = Harness::start("/home/user", "/tmp");
    assert_eq!(harness.view().split_percent, 50);
    assert_eq!(harness.view().log_rows, 6);

    harness.press("Alt+Left");
    assert_eq!(harness.view().split_percent, 45);
    for _ in 0..20 {
        harness.press("Alt+Right");
    }
    assert_eq!(harness.view().split_percent, 90);
    for _ in 0..30 {
        harness.press("Alt+Left");
    }
    assert_eq!(harness.view().split_percent, 10);

    harness.press("Alt+Up");
    assert_eq!(harness.view().log_rows, 7);
    for _ in 0..40 {
        harness.press("Alt+Down");
    }
    assert_eq!(harness.view().log_rows, 1);
    for _ in 0..40 {
        harness.press("Alt+Up");
    }
    assert_eq!(harness.view().log_rows, 30);
}

// Reload and watching

#[test]
fn reload_reads_again_without_emptying_and_the_cursor_follows_the_entry() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b"), file("c")]);
    harness.press("End");
    harness.take_effects();

    harness.press("Y");

    let effects = harness.take_effects();
    assert_eq!(
        effects,
        vec![Effect::ReadDir {
            side: Side::Left,
            location: Location::new(PathBuf::from("/home/user")),
            request: harness.latest_request(Side::Left),
        }]
    );
    assert_eq!(harness.names(Side::Left), ["..", "a", "b", "c"]);

    harness.load(Side::Left, vec![file("a"), file("c"), file("a0")]);
    assert_eq!(harness.cursor_name(Side::Left), "c");
}

#[test]
fn change_in_the_watched_directory_reloads_it() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("End");
    harness.press("Space");
    harness.take_effects();

    harness.changed(Side::Left);

    let effects = harness.take_effects();
    assert_eq!(
        effects,
        vec![Effect::ReadDir {
            side: Side::Left,
            location: Location::new(PathBuf::from("/home/user")),
            request: harness.latest_request(Side::Left),
        }]
    );
    harness.load(Side::Left, vec![file("a"), file("c")]);
    assert_eq!(harness.names(Side::Left), ["..", "a", "c"]);
    assert_eq!(harness.cursor_name(Side::Left), "c");
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
}

#[test]
fn change_of_a_stale_path_is_ignored() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![]);
    harness.take_effects();

    harness.send(Msg::Io(IoEvent::LocationChanged {
        side: Side::Left,
        path: PathBuf::from("/somewhere/else"),
    }));

    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn change_during_a_move_is_applied_after_it_finishes() {
    let mut harness = Harness::start_both_pending(Os::Linux, "/home/user", "/tmp");
    harness.take_effects();

    // The watch already points at the target, so the change names it.
    harness.send(Msg::Io(IoEvent::LocationChanged {
        side: Side::Left,
        path: PathBuf::from("/home/user"),
    }));
    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.app().pane(Side::Left).reload_pending());

    harness.load(Side::Left, vec![file("a")]);
    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(PathBuf::from("/home/user")),
                request: RequestId(5),
            },
            Effect::SetTitle("wafu - /home/user".to_owned()),
        ]
    );
    assert!(!harness.app().pane(Side::Left).reload_pending());
    assert_eq!(harness.names(Side::Left), ["..", "a"]);
}

#[test]
fn watch_failure_is_logged() {
    let mut harness = Harness::start("/home/user", "/tmp");

    harness.send(Msg::Io(IoEvent::WatchFailed {
        side: Side::Left,
        path: PathBuf::from("/home/user"),
        error: IoError::new("too many watches"),
    }));

    assert_eq!(
        harness.log(),
        vec!["cannot watch /home/user: too many watches"]
    );
}

// Mouse

#[test]
fn click_activates_the_pane_and_moves_the_cursor() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Right, vec![file("a"), file("b")]);

    harness.click(Side::Right, 2);

    assert_eq!(harness.app().active(), Side::Right);
    assert_eq!(harness.cursor_name(Side::Right), "b");
    assert_eq!(harness.view().right.cursor, Some(2));
}

#[test]
fn click_past_the_last_row_only_activates_the_pane() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Right, vec![file("a")]);

    harness.click(Side::Right, 5);

    assert_eq!(harness.app().active(), Side::Right);
    assert_eq!(harness.cursor_name(Side::Right), "..");
}

#[test]
fn double_click_opens_the_row() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![dir("docs")]);

    harness.double_click(Side::Left, 1);
    harness.load(Side::Left, vec![]);

    let docs = PathBuf::from("/home/user").join("docs");
    assert_eq!(harness.view().left.path, docs.display().to_string());
}

#[test]
fn right_click_moves_the_cursor_only() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![dir("docs")]);
    harness.take_effects();

    harness.send(Msg::Mouse(MouseInput::RightClick {
        side: Side::Left,
        row: 1,
    }));

    assert_eq!(harness.cursor_name(Side::Left), "docs");
    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn wheel_scrolls_without_moving_the_cursor_and_a_key_brings_it_back() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b"), file("c"), file("d")]);
    harness.viewport(Side::Left, 2);

    harness.wheel(Side::Left, 3);
    assert_eq!(harness.view().left.scroll_top, 3);
    assert_eq!(harness.view().left.cursor, Some(0));

    harness.press("F1");
    assert_eq!(harness.view().left.scroll_top, 0);
}

#[test]
fn wheel_on_the_inactive_pane_is_not_undone_by_keys() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(
        Side::Right,
        vec![file("a"), file("b"), file("c"), file("d")],
    );
    harness.viewport(Side::Right, 2);

    harness.wheel(Side::Right, 2);
    harness.press("Down");

    assert_eq!(harness.view().right.scroll_top, 2);
    assert_eq!(harness.app().active(), Side::Left);
}

#[test]
fn wheel_over_the_log_scrolls_it() {
    let mut harness = Harness::start("/a/b/c/d/e", "/tmp");
    for _ in 0..4 {
        harness.press("Alt+Down");
    }
    for _ in 0..4 {
        harness.press("BS");
        harness.fail_load(Side::Left, "boom");
    }
    assert_eq!(harness.view().log.scroll_top, 2);

    harness.send(Msg::Mouse(MouseInput::LogWheel { rows: -3 }));
    assert_eq!(harness.view().log.scroll_top, 0);
    harness.send(Msg::Mouse(MouseInput::LogWheel { rows: 1 }));
    assert_eq!(harness.view().log.scroll_top, 1);
    assert_eq!(harness.view().left.scroll_top, 0);
}

#[test]
fn mouse_is_ignored_while_a_popup_is_open() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);

    harness.press("S");
    harness.click(Side::Left, 2);

    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert!(harness.popup().is_some());
}

// Navigation

#[test]
fn start_with_a_file_argument_opens_its_parent_with_the_cursor_on_it() {
    let mut harness = Harness::start_unresolved(Os::Linux, "/home/user/notes.txt", "/tmp");

    harness.resolve(Side::Left, "/home/user", Some("notes.txt"));
    harness.load(Side::Left, vec![file("a"), file("notes.txt")]);

    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.cursor_name(Side::Left), "notes.txt");
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn start_with_a_missing_path_falls_back_to_an_ancestor_and_logs_it() {
    let mut harness = Harness::start_unresolved(Os::Linux, "/home/user/gone/deeper", "/tmp");

    harness.resolve(Side::Left, "/home/user", None);
    harness.load(Side::Left, vec![]);

    assert_eq!(
        harness.log(),
        ["/home/user/gone/deeper is not available; moved to /home/user"]
    );
    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.history(Side::Left), ["/home/user"]);
}

#[test]
fn start_with_nothing_existing_falls_back_to_home() {
    let mut harness = Harness::start_unresolved(Os::Linux, "/nowhere", "/tmp");
    harness.take_effects();

    harness.fail_resolve(Side::Left);

    assert_eq!(
        harness.log(),
        ["/nowhere is not available; moved to /home/user"]
    );
    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::Watch {
                side: Side::Left,
                path: PathBuf::from(HOME),
            },
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(PathBuf::from(HOME)),
                request: harness.latest_request(Side::Left),
            },
        ]
    );
}

#[test]
fn a_move_to_a_drive_with_nothing_on_it_stays_put() {
    let mut harness = Harness::start_on(Os::Windows, "/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.press("Ctrl+Right");
    harness.drives(DrivePurpose::Next, &['D']);
    harness.take_effects();

    harness.fail_resolve(Side::Left);

    assert_eq!(harness.log(), ["D:\\ is not available"]);
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Watch {
            side: Side::Left,
            path: PathBuf::from("/home/user"),
        }]
    );
    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.names(Side::Left), ["..", "a"]);
    assert!(harness.view().popup.is_none());
}

#[test]
fn loading_popup_names_every_pane_being_read() {
    let harness = Harness::start_both_pending(Os::Linux, "/home/user", "/tmp");

    let popup = harness.view().popup.unwrap();

    assert_eq!(popup.message, Some("Reading /home/user, /tmp".to_owned()));
    assert!(popup.progress);
    assert_eq!(popup.fields, Vec::new());
}

#[test]
fn history_back_and_forward_move_without_recording_and_stop_at_the_ends() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![dir("docs")]);
    harness.press("Down");
    harness.press("Enter");
    harness.load(Side::Left, vec![]);
    let docs = PathBuf::from("/home/user").join("docs");
    assert_eq!(
        harness.history(Side::Left),
        ["/home/user".to_owned(), docs.display().to_string()]
    );
    harness.take_effects();

    harness.press("Ctrl+Shift+Left");
    let effects = harness.take_effects();
    assert_eq!(
        effects,
        vec![
            Effect::Watch {
                side: Side::Left,
                path: PathBuf::from("/home/user"),
            },
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(PathBuf::from("/home/user")),
                request: harness.latest_request(Side::Left),
            },
        ]
    );
    harness.load(Side::Left, vec![dir("docs")]);
    assert_eq!(harness.view().left.path, "/home/user");
    // Moving through the history does not restore the cursor.
    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert_eq!(harness.history(Side::Left).len(), 2);
    assert_eq!(harness.app().pane(Side::Left).history().pos(), 0);
    harness.take_effects();

    harness.press("Ctrl+Shift+Left");
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("Ctrl+Shift+Right");
    harness.load(Side::Left, vec![]);
    assert_eq!(harness.view().left.path, docs.display().to_string());
    assert_eq!(harness.app().pane(Side::Left).history().pos(), 1);
    harness.take_effects();

    harness.press("Ctrl+Shift+Right");
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn failed_history_move_keeps_the_position() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.press("BS");
    harness.load(Side::Left, vec![]);
    assert_eq!(harness.app().pane(Side::Left).history().pos(), 1);

    harness.press("Ctrl+Shift+Left");
    harness.fail_load(Side::Left, "gone");

    assert_eq!(harness.view().left.path, "/home");
    assert_eq!(harness.app().pane(Side::Left).history().pos(), 1);
    assert_eq!(harness.history(Side::Left), ["/home/user", "/home"]);
}

#[test]
fn a_visit_after_going_back_drops_the_forward_side() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.press("BS");
    harness.load(Side::Left, vec![]);
    harness.press("Ctrl+Shift+Left");
    harness.load(Side::Left, vec![]);
    harness.take_effects();

    harness.press("~");
    assert_eq!(harness.take_effects(), vec![]);
    harness.press("/");
    harness.load(Side::Left, vec![]);

    assert_eq!(harness.history(Side::Left), ["/home/user", "/"]);
}

#[test]
fn history_popup_lists_newest_first_and_picks_by_key() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.press("BS");
    harness.load(Side::Left, vec![]);
    harness.press("/");
    harness.load(Side::Left, vec![]);
    harness.take_effects();

    harness.press("H");

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "History");
    assert_eq!(harness.popup_labels(), ["/", "/home", "/home/user"]);
    let keys: Vec<&str> = popup
        .items
        .iter()
        .map(|item| item.detail.as_str())
        .collect();
    assert_eq!(keys, ["0", "1", "2"]);
    assert_eq!(popup.cursor, Some(0));

    // Picking the current place just closes the popup.
    harness.press("Enter");
    assert_eq!(harness.popup(), None);
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("H");
    harness.press("2");
    assert_eq!(harness.popup(), None);
    harness.load(Side::Left, vec![]);
    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.history(Side::Left).len(), 3);
    assert_eq!(harness.app().pane(Side::Left).history().pos(), 0);

    harness.press("H");
    assert_eq!(harness.view().popup.unwrap().cursor, Some(2));
    harness.press("Down");
    assert_eq!(harness.view().popup.unwrap().cursor, Some(0));
    harness.press("Up");
    harness.press("Up");
    assert_eq!(harness.view().popup.unwrap().cursor, Some(1));
    harness.press("Esc");
    assert_eq!(harness.popup(), None);
}

#[test]
fn going_to_the_current_place_does_nothing() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("~");
    harness.press("Shift+O");
    harness.press("B");
    harness.edit("swap_panes");
    harness.press("Enter");

    assert_eq!(harness.marks(Side::Left), ["a"]);
    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::Watch {
                side: Side::Right,
                path: PathBuf::from("/home/user"),
            },
            Effect::ReadDir {
                side: Side::Right,
                location: Location::new(PathBuf::from("/home/user")),
                request: harness.latest_request(Side::Right),
            },
        ]
    );
    harness.load(Side::Right, vec![]);
    harness.take_effects();

    harness.press("B");
    harness.edit("swap_panes");
    harness.press("Enter");
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn go_root_goes_to_the_top_of_the_path() {
    let mut harness = Harness::start("/home/user", "/tmp");

    harness.press("/");
    harness.load(Side::Left, vec![]);

    assert_eq!(harness.view().left.path, "/");
    assert_eq!(harness.history(Side::Left), ["/home/user", "/"]);
}

#[test]
fn go_to_path_starts_with_the_current_path_and_expands_the_input() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("Shift+J");
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Go to path");
    assert_eq!(popup.fields[0].text, "/home/user");

    harness.edit("~/docs");
    harness.press("Enter");

    let docs = PathBuf::from(HOME).join("docs");
    assert_eq!(harness.popup(), None);
    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::Watch {
                side: Side::Left,
                path: docs.clone(),
            },
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(docs),
                request: harness.latest_request(Side::Left),
            },
        ]
    );
}

#[test]
fn go_to_path_resolves_relative_input_from_the_pane() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("Shift+J");
    harness.edit("docs/../other");
    harness.press("Enter");
    harness.load(Side::Left, vec![]);

    // Compared as paths: folding rebuilds the path with the platform separator.
    assert_eq!(
        harness.app().pane(Side::Left).location().base,
        PathBuf::from("/home/user").join("other")
    );
}

#[test]
fn go_to_path_with_blank_input_just_closes() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("Shift+J");
    harness.edit("   ");
    harness.press("Enter");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn go_to_path_to_a_missing_place_logs_the_read_error_and_stays() {
    let mut harness = Harness::start("/home/user", "/tmp");

    harness.press("Shift+J");
    harness.edit("/nowhere");
    harness.press("Enter");
    harness.fail_load(Side::Left, "No such file or directory");

    assert_eq!(
        harness.log(),
        ["cannot read /nowhere: No such file or directory"]
    );
    assert_eq!(harness.view().left.path, "/home/user");
}

#[test]
fn sync_commands_copy_the_location_across() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("Shift+O");
    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::Watch {
                side: Side::Right,
                path: PathBuf::from("/home/user"),
            },
            Effect::ReadDir {
                side: Side::Right,
                location: Location::new(PathBuf::from("/home/user")),
                request: harness.latest_request(Side::Right),
            },
        ]
    );
    harness.press("Esc");

    harness.press("O");
    harness.load(Side::Left, vec![]);
    assert_eq!(harness.view().left.path, "/tmp");
    assert_eq!(harness.view().right.path, "/tmp");
    assert_eq!(harness.history(Side::Left), ["/home/user", "/tmp"]);
}

#[test]
fn swap_panes_exchanges_the_locations_and_keeps_the_active_pane() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.press("Right");
    harness.take_effects();

    harness.press("B");
    harness.edit("swap_panes");
    harness.press("Enter");

    let effects = harness.take_effects();
    assert_eq!(
        effects,
        vec![
            Effect::Watch {
                side: Side::Left,
                path: PathBuf::from("/tmp"),
            },
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(PathBuf::from("/tmp")),
                request: harness.latest_request(Side::Left),
            },
            Effect::Watch {
                side: Side::Right,
                path: PathBuf::from("/home/user"),
            },
            Effect::ReadDir {
                side: Side::Right,
                location: Location::new(PathBuf::from("/home/user")),
                request: harness.latest_request(Side::Right),
            },
        ]
    );
    harness.load(Side::Left, vec![]);
    harness.load(Side::Right, vec![]);
    assert_eq!(harness.view().left.path, "/tmp");
    assert_eq!(harness.view().right.path, "/home/user");
    assert_eq!(harness.app().active(), Side::Right);
    assert_eq!(harness.history(Side::Right), ["/tmp", "/home/user"]);
}

#[test]
fn a_vanished_place_is_left_for_the_nearest_ancestor() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.take_effects();

    harness.changed(Side::Left);
    let read = harness.latest_request(Side::Left);
    harness.fail_load(Side::Left, "No such file or directory");

    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(PathBuf::from("/home/user")),
                request: read,
            },
            Effect::Resolve {
                side: Side::Left,
                path: PathBuf::from("/home/user"),
                request: harness.latest_resolve(Side::Left),
            },
        ]
    );
    assert_eq!(harness.names(Side::Left), ["..", "a"]);

    harness.resolve(Side::Left, "/home", None);

    assert_eq!(
        harness.log(),
        ["/home/user is not available; moved to /home"]
    );
    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::Watch {
                side: Side::Left,
                path: PathBuf::from("/home"),
            },
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(PathBuf::from("/home")),
                request: harness.latest_request(Side::Left),
            },
        ]
    );
    harness.load(Side::Left, vec![]);
    assert_eq!(harness.view().left.path, "/home");
    assert_eq!(harness.history(Side::Left), ["/home/user", "/home"]);
}

#[test]
fn a_failed_reload_of_a_place_that_still_exists_only_logs() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.changed(Side::Left);
    harness.fail_load(Side::Left, "permission denied");
    harness.take_effects();

    harness.resolve(Side::Left, "/home/user", None);

    assert_eq!(harness.log(), ["cannot read /home/user: permission denied"]);
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Watch {
            side: Side::Left,
            path: PathBuf::from("/home/user"),
        }]
    );
    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.names(Side::Left), ["..", "a"]);
    assert!(harness.view().popup.is_none());
}

#[test]
fn opening_a_broken_link_logs_an_error() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(
        Side::Left,
        vec![Entry {
            kind: EntryKind::Symlink { target: None },
            ..file("dead")
        }],
    );
    harness.press("Down");
    harness.take_effects();

    harness.press("Enter");

    assert_eq!(
        harness.log(),
        [format!(
            "cannot open {}: the symbolic link is broken",
            PathBuf::from("/home/user").join("dead").display()
        )]
    );
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn bookmarks_popup_expands_tilde_and_picks_by_key() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.config("[[bookmarks]]\npath = \"~/docs\"\n[[bookmarks]]\npath = \"/var\"\n");
    harness.take_effects();

    harness.press("J");

    let docs = PathBuf::from(HOME).join("docs");
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Bookmarks");
    assert_eq!(popup.message, None);
    assert_eq!(
        harness.popup_labels(),
        [docs.display().to_string(), "/var".to_owned()]
    );
    assert_eq!(popup.cursor, Some(0));

    harness.press("Up");
    assert_eq!(harness.view().popup.unwrap().cursor, Some(1));

    harness.press("1");
    assert_eq!(harness.popup(), None);
    harness.load(Side::Left, vec![]);
    assert_eq!(harness.view().left.path, "/var");
}

#[test]
fn empty_bookmarks_popup_says_so_and_enter_closes_it() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("J");

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.message, Some("No bookmarks".to_owned()));
    assert_eq!(popup.items, vec![]);
    assert_eq!(popup.cursor, None);

    harness.press("Enter");
    assert_eq!(harness.popup(), None);
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn drive_commands_log_on_other_systems() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("L");
    harness.press("Ctrl+Left");

    assert_eq!(
        harness.log(),
        [
            "drives is available only on Windows",
            "drive_prev is available only on Windows"
        ]
    );
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn drives_popup_lists_the_letters_and_picks_by_letter() {
    let mut harness = Harness::start_on(Os::Windows, "/home/user", "/tmp");
    harness.take_effects();

    harness.press("L");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::ListDrives {
            purpose: DrivePurpose::Show,
        }]
    );

    harness.drives(DrivePurpose::Show, &['D', 'C']);
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Drives");
    assert_eq!(harness.popup_labels(), ["C:\\", "D:\\"]);

    harness.press("Shift+D");
    assert_eq!(harness.popup(), None);
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Resolve {
            side: Side::Left,
            path: PathBuf::from("D:\\"),
            request: harness.latest_resolve(Side::Left),
        }]
    );
}

#[test]
fn drive_next_and_prev_without_a_current_drive_go_to_the_ends() {
    let mut harness = Harness::start_on(Os::Windows, "/home/user", "/tmp");
    harness.take_effects();

    harness.press("Ctrl+Right");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::ListDrives {
            purpose: DrivePurpose::Next,
        }]
    );
    harness.drives(DrivePurpose::Next, &['C', 'D']);
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Resolve { path, .. }] if path == &PathBuf::from("C:\\")
    ));
    harness.press("Esc");
    harness.take_effects();

    harness.press("Ctrl+Left");
    harness.drives(DrivePurpose::Prev, &['C', 'D']);
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::ListDrives { .. }, Effect::Resolve { path, .. }] if path == &PathBuf::from("D:\\")
    ));

    harness.press("Esc");
    harness.take_effects();
    harness.press("Ctrl+Right");
    harness.drives(DrivePurpose::Next, &[]);
    assert_eq!(
        harness.take_effects(),
        vec![Effect::ListDrives {
            purpose: DrivePurpose::Next,
        }]
    );
}

#[cfg(windows)]
#[test]
fn drive_switching_wraps_and_remembers_the_last_place_on_each_drive() {
    let mut harness = Harness::start_on(Os::Windows, r"C:\Users\me", r"D:\");
    harness.take_effects();

    // Already on C, so nothing happens.
    harness.press("L");
    harness.drives(DrivePurpose::Show, &['C', 'D']);
    harness.take_effects();
    assert_eq!(harness.view().popup.unwrap().cursor, Some(0));
    harness.press("C");
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("Ctrl+Left");
    harness.drives(DrivePurpose::Prev, &['C', 'D']);
    harness.resolve(Side::Left, r"D:\", None);
    harness.load(Side::Left, vec![]);
    assert_eq!(harness.view().left.path, r"D:\");
    harness.take_effects();

    harness.press("Ctrl+Right");
    harness.drives(DrivePurpose::Next, &['C', 'D']);
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::ListDrives { .. }, Effect::Resolve { path, .. }] if path == &PathBuf::from(r"C:\Users\me")
    ));
}

#[test]
fn rows_carry_their_kind_for_coloring() {
    let mut harness = Harness::start("/home/user", "/tmp");

    harness.load(
        Side::Left,
        vec![
            file("a"),
            dir("d"),
            Entry {
                kind: EntryKind::Symlink {
                    target: Some(TargetKind::Dir),
                },
                ..file("l")
            },
            Entry {
                kind: EntryKind::Symlink { target: None },
                ..file("x")
            },
        ],
    );

    let kinds: Vec<RowKind> = harness
        .view()
        .left
        .rows
        .iter()
        .map(|row| row.kind)
        .collect();
    assert_eq!(
        kinds,
        [
            RowKind::Parent,
            RowKind::Dir,
            RowKind::Link,
            RowKind::File,
            RowKind::Link
        ]
    );
}

#[test]
fn start_takes_paths_from_the_arguments_then_the_state_then_home() {
    let state = StateFile {
        left: PaneState {
            path: Some(PathBuf::from("/saved/left")),
            ..PaneState::default()
        },
        right: PaneState::default(),
        ..StateFile::default()
    };
    let harness = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/arg"),
        None,
        ConfigSource::Text(String::new()),
        Ok(Some(state.clone())),
    ));

    let resolves: Vec<PathBuf> = harness
        .app()
        .pane(Side::Left)
        .loading()
        .into_iter()
        .chain(harness.app().pane(Side::Right).loading())
        .filter_map(|in_flight| match in_flight {
            InFlight::Resolve { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(resolves, [PathBuf::from("/arg"), PathBuf::from(HOME)]);

    let mut saved = Harness::start_from(start_msg_with(
        Os::Linux,
        None,
        None,
        ConfigSource::Text(String::new()),
        Ok(Some(state)),
    ));
    assert!(matches!(
        saved.app().pane(Side::Left).loading(),
        Some(InFlight::Resolve { path, .. }) if path == &PathBuf::from("/saved/left")
    ));
    saved.resolve(Side::Left, "/saved/left", None);
    saved.load(Side::Left, vec![]);
    assert_eq!(saved.history(Side::Left), ["/saved/left"]);
}

#[test]
fn start_restores_the_saved_display_state_and_ignores_initial() {
    let state = StateFile {
        left: PaneState {
            sort: Some("size".to_owned()),
            descending: Some(true),
            dirs_first: Some(false),
            ignore_case: Some(false),
            show_hidden: Some(true),
            history: vec![PathBuf::from("/a"), PathBuf::from("/b")],
            history_pos: Some(0),
            ..PaneState::default()
        },
        right: PaneState {
            sort: Some("no such key".to_owned()),
            ..PaneState::default()
        },
        window: None,
        split_percent: Some(95),
        log_rows: Some(0),
    };
    let mut harness = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Text("[initial]\nsort = \"time\"\nshow_hidden = true\n".to_owned()),
        Ok(Some(state)),
    ));
    harness.resolve(Side::Left, "/home/user", None);
    harness.resolve(Side::Right, "/tmp", None);
    harness.load(Side::Left, vec![]);
    harness.load(Side::Right, vec![]);

    let left = harness.app().pane(Side::Left);
    assert_eq!(
        left.sort(),
        SortSpec {
            key: SortKey::Size,
            descending: true,
            dirs_first: false,
            ignore_case: false,
        }
    );
    assert!(left.show_hidden());
    // The start-up visit follows the restored position.
    assert_eq!(harness.history(Side::Left), ["/a", "/home/user"]);
    let right = harness.app().pane(Side::Right);
    assert_eq!(right.sort().key, SortKey::Time);
    assert!(right.show_hidden());
    let view = harness.view();
    assert_eq!(view.split_percent, 90);
    assert_eq!(view.log_rows, 1);
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn start_without_a_state_file_uses_initial() {
    let mut harness = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Text(
            "[initial]\nsort = \"extension\"\ndescending = true\nshow_hidden = true\n".to_owned(),
        ),
        Ok(None),
    ));
    harness.resolve(Side::Left, "/home/user", None);
    harness.load(Side::Left, vec![]);

    let pane = harness.app().pane(Side::Left);
    assert_eq!(pane.sort().key, SortKey::Extension);
    assert!(pane.sort().descending);
    assert!(pane.show_hidden());
    assert_eq!(
        harness.view().left.info,
        "0 items | 0 marked (0 bytes) | sort: extension desc | hidden: on"
    );
}

#[test]
fn start_logs_how_the_configuration_and_state_were_found() {
    let created = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Created,
        Ok(None),
    ));
    assert_eq!(created.log(), [format!("created {CONFIG_PATH}")]);

    let unreadable = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Unreadable(IoError::new("permission denied")),
        Err(StateFile::from_toml("[left\n").unwrap_err()),
    ));
    let log = unreadable.log();
    assert_eq!(log.len(), 2);
    assert_eq!(
        log[0],
        format!("cannot read {CONFIG_PATH}: permission denied")
    );
    assert!(log[1].starts_with("cannot read the state file: "));

    let broken = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Text("[general]\nmkdir_then_enter =\n".to_owned()),
        Ok(None),
    ));
    let log = broken.log();
    assert_eq!(log.len(), 1);
    assert!(log[0].starts_with(&format!("config: {CONFIG_PATH}:2: ")));
    assert!(broken.app().config().general.mkdir_then_enter);

    let warned = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Text("[general]\nfoo = 1\n".to_owned()),
        Ok(None),
    ));
    assert_eq!(warned.log(), ["config: unknown item general.foo"]);
}

#[test]
fn reloading_the_configuration_changes_keys_and_style_but_not_initial() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.config(
        "[appearance]\ntheme = \"dark\"\nellipsis = \"middle\"\nbogus = 1\n[appearance.font]\nsize = 16\n[initial]\nshow_hidden = true\n[keys.filelist]\n\"Q\" = \"nop\"\n\"F2\" = \"go_root\"\n",
    );

    assert_eq!(
        harness.log(),
        ["config: unknown item appearance.bogus", "config reloaded",]
    );
    let style = harness.view().style;
    assert_eq!(style.theme, Theme::Dark);
    assert_eq!(style.ellipsis, Ellipsis::Middle);
    assert_eq!(style.fonts.filelist.size, 16);
    assert!(!harness.app().pane(Side::Left).show_hidden());
    assert_eq!(harness.marks(Side::Left), ["a"]);

    harness.press("Q");
    assert_eq!(harness.take_effects(), vec![]);
    harness.press("F2");
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Watch { .. }, Effect::ReadDir { location, .. }] if location.base == Path::new("/")
    ));
}

#[test]
fn a_syntax_error_on_reload_keeps_the_previous_configuration() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.config("[appearance]\ntheme = \"light\"\n");
    harness.take_effects();

    harness.config("[appearance\n");

    let log = harness.log();
    assert_eq!(log.len(), 2);
    assert_eq!(log[0], "config reloaded");
    assert!(log[1].starts_with(&format!("config: {CONFIG_PATH}:1: ")));
    assert_eq!(harness.view().style.theme, Theme::Light);

    harness.send(Msg::Io(IoEvent::ConfigLoaded {
        result: Err(IoError::new("gone")),
    }));
    assert_eq!(harness.log()[2], format!("cannot read {CONFIG_PATH}: gone"));
    assert_eq!(harness.view().style.theme, Theme::Light);
}

#[test]
fn quit_saves_the_state_and_shift_q_does_not() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.press("Space");
    harness.press("S");
    harness.press("S");
    harness.press("Alt+Right");
    harness.press("Alt+Up");
    harness.take_effects();

    harness.press("Q");

    let state = StateFile {
        left: PaneState {
            path: Some(PathBuf::from("/home/user")),
            sort: Some("size".to_owned()),
            descending: Some(false),
            dirs_first: Some(true),
            ignore_case: Some(true),
            show_hidden: Some(false),
            history: vec![PathBuf::from("/home/user")],
            history_pos: Some(0),
        },
        right: PaneState {
            path: Some(PathBuf::from("/tmp")),
            sort: Some("name".to_owned()),
            descending: Some(false),
            dirs_first: Some(true),
            ignore_case: Some(true),
            show_hidden: Some(false),
            history: vec![PathBuf::from("/tmp")],
            history_pos: Some(0),
        },
        window: None,
        split_percent: Some(55),
        log_rows: Some(7),
    };
    assert_eq!(
        harness.take_effects(),
        vec![Effect::SaveState(state), Effect::Quit]
    );

    harness.press("Shift+Q");
    assert_eq!(harness.take_effects(), vec![Effect::Quit]);

    harness.send(Msg::CloseRequested);
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::SaveState(_), Effect::Quit]
    ));
}

#[test]
fn a_pane_without_a_place_saves_no_path() {
    let mut harness = Harness::start_unresolved(Os::Linux, "/home/user", "/tmp");
    harness.press("Esc");
    harness.take_effects();

    harness.send(Msg::CloseRequested);

    let effects = harness.take_effects();
    let [Effect::SaveState(state), Effect::Quit] = effects.as_slice() else {
        panic!("expected SaveState and Quit");
    };
    assert_eq!(state.left.path, None);
    assert_eq!(state.left.history, Vec::<PathBuf>::new());
}

#[test]
fn new_window_hands_over_the_state_and_logs_a_failure() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.press("Space");
    harness.take_effects();

    harness.press("Alt+Z");

    let effects = harness.take_effects();
    let [Effect::SpawnNewWindow(state)] = effects.as_slice() else {
        panic!("expected SpawnNewWindow");
    };
    assert_eq!(state.left.path, Some(PathBuf::from("/home/user")));
    assert_eq!(state.right.path, Some(PathBuf::from("/tmp")));
    assert_eq!(state.window, None);

    harness.send(Msg::Io(IoEvent::Spawned {
        kind: SpawnKind::NewWindow,
        error: None,
    }));
    assert_eq!(harness.log(), Vec::<String>::new());
    harness.send(Msg::Io(IoEvent::Spawned {
        kind: SpawnKind::NewWindow,
        error: Some(IoError::new("no exe")),
    }));
    assert_eq!(harness.log(), ["cannot start a new window: no exe"]);
}

#[test]
fn external_commands_expand_placeholders_and_report_their_start() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.config(
        "[commands.term]\nrun = \"wt -d {dir} {paths} --other {other_dir} {cursor_path}\"\n[keys.filelist]\n\"F2\" = \"$term\"\n",
    );
    harness.load(Side::Left, vec![file("a"), file("b"), file("c")]);
    harness.press("Down");
    harness.press("Space");
    harness.press("Space");
    harness.take_effects();

    harness.press("F2");

    let home = PathBuf::from(HOME);
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Spawn {
            program: "wt".to_owned(),
            args: vec![
                OsString::from("-d"),
                home.as_os_str().to_owned(),
                home.join("a").into_os_string(),
                home.join("b").into_os_string(),
                OsString::from("--other"),
                OsString::from("/tmp"),
                home.join("c").into_os_string(),
            ],
            cwd: home.clone(),
        }]
    );

    harness.send(Msg::Io(IoEvent::Spawned {
        kind: SpawnKind::External {
            program: "wt".to_owned(),
        },
        error: None,
    }));
    harness.send(Msg::Io(IoEvent::Spawned {
        kind: SpawnKind::External {
            program: "wt".to_owned(),
        },
        error: Some(IoError::new("not found")),
    }));
    assert_eq!(
        harness.log()[1..],
        ["started wt", "cannot start wt: not found"]
    );
}

#[test]
fn external_commands_run_from_the_palette_with_no_targets() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.config("[commands.term]\nrun = \"wt {paths}\"\n");
    harness.take_effects();

    harness.press("B");
    harness.edit("$te");
    assert_eq!(harness.popup_labels(), ["$term"]);
    harness.press("Enter");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::Spawn {
            program: "wt".to_owned(),
            args: vec![],
            cwd: PathBuf::from(HOME),
        }]
    );
}

#[test]
fn edit_config_opens_the_configuration_in_the_editor() {
    let mut harness = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Text(
            "[general]\neditor = \"code -w {paths}\"\n[os.macos.general]\neditor = \"open -t {paths}\"\n".to_owned(),
        ),
        Ok(None),
    ));
    harness.resolve(Side::Left, "/home/user", None);
    harness.resolve(Side::Right, "/tmp", None);
    harness.load(Side::Left, vec![]);
    harness.load(Side::Right, vec![]);
    harness.take_effects();

    harness.press("Z");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::Spawn {
            program: "code".to_owned(),
            args: vec![OsString::from("-w"), OsString::from(CONFIG_PATH)],
            cwd: PathBuf::from(HOME),
        }]
    );

    let default = Harness::start_on(Os::MacOs, "/home/user", "/tmp");
    assert_eq!(
        default.app().config().general.editor,
        CommandLine::parse("open -t {paths}").unwrap()
    );
}
