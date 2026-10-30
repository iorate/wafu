use std::path::{MAIN_SEPARATOR, Path, PathBuf};

use super::*;
use crate::config::Theme;
use crate::entry::{
    Directory, Entry, EntryKind, LocalDateTime, SpecialKind, TargetKind, Timestamp,
};
use crate::external::CommandLine;
use crate::listing::{SortKey, SortSpec};
use crate::pane::InFlight;
use crate::state::{PaneState, StateFile};
use crate::test_support::{CONFIG_PATH, HOME, Harness, dir, file, special, start_msg_with};
use crate::view::{ItemMark, RowKind};

mod archives;
mod operations;
mod preview;

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
                climb: true,
                request: RequestId(1),
            },
            Effect::Resolve {
                side: Side::Right,
                path: PathBuf::from("/tmp"),
                climb: true,
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
        .map(|row| (row.name.as_str(), row.extension.as_str(), row.marker))
        .collect();
    assert_eq!(
        columns,
        [
            ("..", "", ""),
            ("conf.d", "", "/"),
            ("a.tar", ".gz", ""),
            ("noext", "", ""),
        ]
    );
}

#[test]
fn marker_column_follows_ls_f_and_directories_and_special_files_have_no_size() {
    let mut harness = Harness::start("/home/user", "/tmp");
    let executable = Entry {
        kind: EntryKind::File { executable: true },
        ..file("run")
    };

    harness.load(
        Side::Left,
        vec![
            dir("d"),
            executable,
            file("plain"),
            special("fifo", SpecialKind::Fifo),
            special("sock", SpecialKind::Socket),
            special("tty", SpecialKind::CharDevice),
        ],
    );

    let view = harness.view();
    let columns: Vec<(&str, &str, &str)> = view
        .left
        .rows
        .iter()
        .map(|row| (row.name.as_str(), row.marker, row.size.as_str()))
        .collect();
    assert_eq!(
        columns,
        [
            ("..", "", ""),
            ("d", "/", ""),
            ("fifo", "|", ""),
            ("plain", "", "0"),
            ("run", "*", "0"),
            ("sock", "=", ""),
            ("tty", "", ""),
        ]
    );
}

#[test]
fn status_line_spells_out_the_cursor_entry_of_the_active_pane() {
    use crate::entry::Permissions;

    let mut harness = Harness::start("/home/user", "/tmp");
    let plain = Entry {
        size: 1234,
        modified: Some(timestamp()),
        permissions: Some(Permissions::Mode(0o644)),
        ..file("a_long_name.txt")
    };
    let link = Entry {
        kind: EntryKind::Symlink {
            target: Some(TargetKind::File { executable: false }),
        },
        size: 42,
        permissions: Some(Permissions::Mode(0o755)),
        link_target: Some(std::ffi::OsString::from("target.txt")),
        ..file("link.txt")
    };
    let broken = Entry {
        kind: EntryKind::Symlink { target: None },
        permissions: Some(Permissions::Mode(0o755)),
        ..file("broken")
    };
    harness.load(Side::Left, vec![plain, link, broken, dir("sub")]);

    assert_eq!(harness.view().status, "..");

    let mut statuses = Vec::new();
    for _ in 0..4 {
        harness.press("Down");
        statuses.push(harness.view().status);
    }
    statuses.sort();

    assert_eq!(
        statuses,
        [
            "a_long_name.txt | -rw-r--r-- | 1234 bytes | 2026/01/02 03:04:05",
            "broken | lrwxr-xr-x | ? | ?",
            "link.txt -> target.txt | lrwxr-xr-x | 42 bytes",
            "sub/ | d?????????",
        ]
    );

    harness.press("Tab");
    assert_eq!(harness.view().status, "..");
}

#[test]
fn status_line_marks_names_and_link_targets_like_ls_lf() {
    let mut harness = Harness::start("/home/user", "/tmp");
    let link = |name: &str, target: &str, kind: Option<TargetKind>| Entry {
        kind: EntryKind::Symlink { target: kind },
        link_target: Some(std::ffi::OsString::from(target)),
        ..file(name)
    };
    harness.load(
        Side::Left,
        vec![
            Entry {
                kind: EntryKind::File { executable: true },
                ..file("run")
            },
            special("fifo", SpecialKind::Fifo),
            special("sock", SpecialKind::Socket),
            special("tty", SpecialKind::CharDevice),
            link("to_dir", "sub", Some(TargetKind::Dir)),
            link("to_run", "run", Some(TargetKind::File { executable: true })),
            link(
                "to_fifo",
                "fifo",
                Some(TargetKind::Special(SpecialKind::Fifo)),
            ),
            link("broken", "missing", None),
            Entry {
                kind: EntryKind::Symlink {
                    target: Some(TargetKind::File { executable: false }),
                },
                ..file("in_archive")
            },
        ],
    );

    let mut statuses = Vec::new();
    for _ in 0..9 {
        harness.press("Down");
        statuses.push(harness.view().status);
    }
    statuses.sort();

    assert_eq!(
        statuses,
        [
            "broken -> missing | l????????? | ? | ?",
            "fifo| | p?????????",
            "in_archive | l????????? | 0 bytes",
            "run* | -????????? | 0 bytes",
            "sock= | s?????????",
            "to_dir -> sub/ | l?????????",
            "to_fifo -> fifo| | l????????? | 0 bytes",
            "to_run -> run* | l????????? | 0 bytes",
            "tty | c?????????",
        ]
    );
}

#[test]
fn status_line_is_empty_for_an_empty_root() {
    let mut harness = Harness::start("/", "/tmp");
    harness.load(Side::Left, vec![]);

    assert_eq!(harness.view().status, "");
}

#[test]
fn status_line_on_the_parent_row_spells_out_the_directory_itself() {
    use crate::entry::Permissions;

    let mut harness = Harness::start("/home/user", "/tmp");
    let own = Entry {
        modified: Some(timestamp()),
        permissions: Some(Permissions::Mode(0o755)),
        ..dir("user")
    };
    harness.load_directory(
        Side::Left,
        Directory {
            own: Some(own),
            entries: vec![file("a")],
        },
    );

    assert_eq!(
        harness.view().status,
        "../ | drwxr-xr-x | 2026/01/02 03:04:05"
    );
    let row = &harness.view().left.rows[0];
    assert_eq!(
        (
            row.name.as_str(),
            row.marker,
            row.size.as_str(),
            row.modified.as_str()
        ),
        ("..", "/", "", "2026/01/02 03:04:05")
    );
    assert_eq!(row.kind, RowKind::Dir);

    harness.press("Down");
    assert_eq!(harness.view().status, "a | -????????? | 0 bytes");
}

#[test]
fn status_line_on_the_parent_row_of_an_archive_root_describes_the_archive_file() {
    use crate::entry::Permissions;

    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a.zip")]);
    harness.press("Down");
    harness.press("Enter");
    let own = Entry {
        size: 1234,
        permissions: Some(Permissions::Mode(0o644)),
        ..file("a.zip")
    };
    harness.load_directory(
        Side::Left,
        Directory {
            own: Some(own),
            entries: vec![],
        },
    );

    assert_eq!(harness.view().status, ".. | -rw-r--r-- | 1234 bytes");
    let row = &harness.view().left.rows[0];
    assert_eq!(
        (row.marker, row.size.as_str(), row.modified.as_str()),
        ("", "1234", "")
    );
}

#[test]
fn status_line_on_the_parent_row_inside_a_link_shows_the_link() {
    use crate::entry::Permissions;

    let mut harness = Harness::start("/tmp", "/home/user");
    let own = Entry {
        kind: EntryKind::Symlink {
            target: Some(TargetKind::Dir),
        },
        permissions: Some(Permissions::Mode(0o755)),
        link_target: Some(std::ffi::OsString::from("private/tmp")),
        ..file("tmp")
    };
    harness.load_directory(
        Side::Left,
        Directory {
            own: Some(own),
            entries: vec![],
        },
    );

    assert_eq!(harness.view().status, ".. -> private/tmp/ | lrwxr-xr-x");
    assert_eq!(harness.view().left.rows[0].marker, "@");
    assert_eq!(harness.view().left.rows[0].kind, RowKind::Link);
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
            target: Some(TargetKind::File { executable: false }),
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
                row.marker,
                row.size.as_str(),
                row.modified.as_str(),
            )
        })
        .collect();
    assert_eq!(
        columns,
        [
            ("..", "", "", ""),
            ("to_dir", "@", "", ""),
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
        result: Ok(Directory {
            own: None,
            entries: vec![file("stale")],
        }),
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
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn focus_left_when_already_left_does_nothing() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("Left");

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.app().active(), Side::Left);
    assert_eq!(harness.log(), Vec::<String>::new());
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
fn new_window_is_dropped_while_a_move_is_in_progress() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.press("BS");
    harness.take_effects();

    harness.press("Alt+Z");

    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.app().pane(Side::Left).loading().is_some());
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
        result: Ok(Directory {
            own: None,
            entries: vec![dir("user")],
        }),
    }));
    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn escape_with_both_panes_moving_cancels_only_the_earliest_move() {
    let mut harness = Harness::start_both_pending(Os::Linux, "/home/user", "/tmp");
    let left = harness.latest_request(Side::Left);
    harness.take_effects();

    harness.press("Esc");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::CancelRead { request: left }]
    );
    assert!(harness.app().pane(Side::Left).loading().is_none());
    assert_eq!(
        harness.view().popup.unwrap().message,
        Some("Reading /tmp".to_owned())
    );

    harness.load(Side::Right, vec![file("a")]);

    assert_eq!(harness.view().left.path, "");
    assert_eq!(harness.view().right.path, "/tmp");
    assert_eq!(harness.names(Side::Right), ["..", "a"]);
    assert!(harness.view().popup.is_none());
}

#[test]
fn escape_cancels_the_move_requested_first_even_when_the_other_pane_moved_on() {
    let mut harness = Harness::start_unresolved(Os::Linux, "/home/user", "/tmp");
    harness.resolve(Side::Left, "/home/user", None);
    let right = harness.latest_resolve(Side::Right);
    assert_eq!(
        harness.view().popup.unwrap().message,
        Some("Reading /tmp".to_owned())
    );
    harness.take_effects();

    harness.press("Esc");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::CancelRead { request: right }]
    );
    assert!(harness.app().pane(Side::Right).loading().is_none());

    harness.load(Side::Left, vec![file("a")]);

    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(harness.view().right.path, "");
    assert!(harness.view().popup.is_none());
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

    harness.press("F3");

    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.view().popup.is_none());
    assert_eq!(harness.log(), Vec::<String>::new());
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
fn mark_glob_with_an_invalid_pattern_is_logged_and_keeps_the_marks() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a.png"), file("b.png")]);
    harness.press("Down");
    harness.press("Space");

    harness.press("@");
    harness.edit("[");
    harness.press("Enter");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.log().len(), 1);
    assert!(harness.log()[0].starts_with("invalid pattern [: "));
    assert_eq!(harness.marks(Side::Left), ["a.png"]);
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

    harness.press("F5");
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
fn clear_filter_without_a_filter_does_nothing_and_keeps_the_marks() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("*");

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.marks(Side::Left), ["a"]);
    assert_eq!(harness.log(), Vec::<String>::new());
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

#[test]
fn jump_without_a_match_does_nothing() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("alpha"), file("beta")]);
    harness.press("Down");
    harness.take_effects();

    harness.press("Ctrl+Z");

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.cursor_name(Side::Left), "alpha");
    assert_eq!(harness.log(), Vec::<String>::new());
}

// Palette

#[test]
fn palette_filters_commands_and_runs_the_selected_one() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.press("Down");
    harness.press("Space");

    harness.press("F1");
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Palette");
    assert_eq!(popup.items.len(), Command::all().count() - 4);
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

    harness.press("F1");
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

    harness.press("F1");
    harness.edit("no such command");
    assert_eq!(harness.view().popup.unwrap().cursor, None);
    harness.press("Enter");
    assert_eq!(harness.popup(), None);

    harness.press("F1");
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

    harness.press("F1");
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

    harness.press("F5");

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
fn reload_lets_keys_and_mouse_through_without_a_progress_popup() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);

    harness.press("F5");

    assert!(harness.view().popup.is_none());
    harness.press("Down");
    assert_eq!(harness.cursor_name(Side::Left), "a");
    harness.click(Side::Left, 2);
    assert_eq!(harness.cursor_name(Side::Left), "b");
    assert!(harness.app().pane(Side::Left).loading().is_some());
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
fn change_in_the_watched_directory_lets_keys_and_mouse_through_without_a_progress_popup() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);

    harness.changed(Side::Left);

    assert!(harness.view().popup.is_none());
    harness.press("Down");
    assert_eq!(harness.cursor_name(Side::Left), "a");
    harness.click(Side::Left, 2);
    assert_eq!(harness.cursor_name(Side::Left), "b");
    assert!(harness.app().pane(Side::Left).loading().is_some());
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
fn double_click_past_the_last_row_only_activates_the_pane() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Right, vec![dir("docs")]);
    harness.click(Side::Right, 1);
    harness.press("Tab");
    harness.take_effects();

    harness.double_click(Side::Right, 5);

    assert_eq!(harness.app().active(), Side::Right);
    assert_eq!(harness.cursor_name(Side::Right), "docs");
    assert_eq!(harness.view().right.path, "/tmp");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::SetTitle("wafu - /tmp".to_owned())]
    );
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

    harness.press("F3");
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
fn scrollbar_drag_scrolls_without_moving_the_cursor_and_stops_at_the_end() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b"), file("c"), file("d")]);
    harness.viewport(Side::Left, 2);

    harness.scroll_to(Side::Left, 2);
    assert_eq!(harness.view().left.scroll_top, 2);
    assert_eq!(harness.view().left.cursor, Some(0));

    harness.scroll_to(Side::Left, 100);
    assert_eq!(harness.view().left.scroll_top, 3);

    harness.scroll_to(Side::Left, 0);
    assert_eq!(harness.view().left.scroll_top, 0);
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
fn scrollbar_drag_over_the_log_scrolls_it_and_the_tail_resumes_following() {
    let mut harness = Harness::start("/a/b/c/d/e", "/tmp");
    for _ in 0..4 {
        harness.press("Alt+Down");
    }
    for _ in 0..4 {
        harness.press("BS");
        harness.fail_load(Side::Left, "boom");
    }
    assert_eq!(harness.view().log.scroll_top, 2);

    harness.send(Msg::Mouse(MouseInput::LogScrollTo { top: 1 }));
    assert_eq!(harness.view().log.scroll_top, 1);
    harness.press("BS");
    harness.fail_load(Side::Left, "boom");
    assert_eq!(harness.view().log.scroll_top, 1);

    harness.send(Msg::Mouse(MouseInput::LogScrollTo { top: 100 }));
    assert_eq!(harness.view().log.scroll_top, 3);
    harness.press("BS");
    harness.fail_load(Side::Left, "boom");
    assert_eq!(harness.view().log.scroll_top, 4);
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
fn loading_popup_names_only_the_earliest_move() {
    let mut harness = Harness::start_both_pending(Os::Linux, "/home/user", "/tmp");

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.message, Some("Reading /home/user".to_owned()));
    assert!(popup.progress);
    assert_eq!(popup.fields, Vec::new());

    harness.load(Side::Left, vec![]);

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.message, Some("Reading /tmp".to_owned()));
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
    assert_eq!(harness.log(), Vec::<String>::new());
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
    harness.load(Side::Left, vec![]);
    assert_eq!(harness.history(Side::Left), ["/home/user", "/home"]);
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
fn go_home_goes_to_the_home_directory() {
    let mut harness = Harness::start("/tmp", "/tmp");
    harness.take_effects();

    harness.press("~");

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
    harness.load(Side::Left, vec![]);
    assert_eq!(harness.view().left.path, HOME);
    assert_eq!(harness.history(Side::Left), ["/tmp", HOME]);
}

#[test]
fn go_home_at_home_reads_it_again_without_recording_history() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("~");

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
                request: harness.latest_request(Side::Left),
            },
        ]
    );
    harness.load(Side::Left, vec![file("a"), file("b")]);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert_eq!(harness.history(Side::Left), ["/home/user"]);
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn sync_to_other_at_the_same_place_reads_it_again_without_recording_history() {
    let mut harness = Harness::start("/home/user", "/home/user");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("O");

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
                request: harness.latest_request(Side::Left),
            },
        ]
    );
    harness.load(Side::Left, vec![file("a"), file("b")]);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert_eq!(harness.history(Side::Left), ["/home/user"]);
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn sync_other_to_this_at_the_same_place_reads_it_again_without_recording_history() {
    let mut harness = Harness::start("/home/user", "/home/user");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("Down");
    harness.press("Space");
    harness.press("Tab");
    harness.take_effects();

    harness.press("Shift+O");

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
                request: harness.latest_request(Side::Left),
            },
        ]
    );
    harness.load(Side::Left, vec![file("a"), file("b")]);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert_eq!(harness.history(Side::Left), ["/home/user"]);
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn swap_panes_at_the_same_place_reads_both_again_without_recording_history() {
    let mut harness = Harness::start("/home/user", "/home/user");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("F1");
    harness.edit("swap_panes");
    harness.press("Enter");

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
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.load(Side::Right, vec![]);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert_eq!(harness.history(Side::Left), ["/home/user"]);
    assert_eq!(harness.history(Side::Right), ["/home/user"]);
    assert_eq!(harness.log(), Vec::<String>::new());
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
        vec![Effect::Resolve {
            side: Side::Left,
            path: docs.clone(),
            climb: false,
            request: harness.latest_resolve(Side::Left),
        }]
    );

    harness.resolve(Side::Left, &docs.to_string_lossy(), None);

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

    // Compared as paths: folding rebuilds the path with the platform separator.
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Resolve { path, .. }] if *path == PathBuf::from("/home/user").join("other")
    ));
}

#[test]
fn go_to_path_to_a_file_opens_its_parent_with_the_cursor_on_it() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("Shift+J");
    harness.edit("/home/user/docs/b.txt");
    harness.press("Enter");
    harness.take_effects();
    harness.resolve(Side::Left, "/home/user/docs", Some("b.txt"));

    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::Watch {
                side: Side::Left,
                path: PathBuf::from("/home/user/docs"),
            },
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(PathBuf::from("/home/user/docs")),
                request: harness.latest_request(Side::Left),
            },
        ]
    );

    harness.load(Side::Left, vec![file("a.txt"), file("b.txt")]);

    assert_eq!(harness.view().left.path, "/home/user/docs");
    assert_eq!(harness.cursor_name(Side::Left), "b.txt");
}

#[test]
fn go_to_path_to_a_file_in_the_current_place_rereads_it_with_the_cursor_on_the_file() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a.txt"), file("b.txt")]);
    harness.take_effects();

    harness.press("Shift+J");
    harness.edit("/home/user/b.txt");
    harness.press("Enter");
    harness.take_effects();
    harness.resolve(Side::Left, "/home/user", Some("b.txt"));

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Watch { .. }, Effect::ReadDir { location, .. }]
            if *location == Location::new(PathBuf::from("/home/user"))
    ));

    harness.load(Side::Left, vec![file("a.txt"), file("b.txt")]);

    assert_eq!(harness.cursor_name(Side::Left), "b.txt");
    assert_eq!(harness.history(Side::Left), ["/home/user"]);
}

#[test]
fn go_to_path_to_the_current_place_reads_it_again_without_recording_history() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a"), file("b")]);
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("Shift+J");
    harness.press("Enter");
    harness.take_effects();
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
                request: harness.latest_request(Side::Left),
            },
        ]
    );
    harness.load(Side::Left, vec![file("a"), file("b")]);
    assert_eq!(harness.marks(Side::Left), Vec::<String>::new());
    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert_eq!(harness.history(Side::Left), ["/home/user"]);
    assert_eq!(harness.log(), Vec::<String>::new());
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
fn go_to_path_to_a_missing_place_logs_an_error_and_stays() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("Shift+J");
    harness.edit("/nowhere");
    harness.press("Enter");
    harness.take_effects();
    harness.fail_resolve(Side::Left);

    assert_eq!(
        harness.log(),
        [format!("{MAIN_SEPARATOR}nowhere does not exist")]
    );
    assert_eq!(harness.view().left.path, "/home/user");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Watch {
            side: Side::Left,
            path: PathBuf::from("/home/user"),
        }]
    );
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

    harness.press("F1");
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
fn sync_to_other_does_nothing_when_the_other_pane_has_no_place() {
    let mut harness = right_pane_without_a_place();

    harness.press("O");

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn sync_other_to_this_does_nothing_when_this_pane_has_no_place() {
    let mut harness = right_pane_without_a_place();
    harness.press("Right");
    harness.take_effects();

    harness.press("Shift+O");

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn swap_panes_does_nothing_when_a_pane_has_no_place() {
    let mut harness = right_pane_without_a_place();

    harness.press("F1");
    harness.edit("swap_panes");
    harness.take_effects();
    harness.press("Enter");

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn moving_gives_a_pane_without_a_place_its_place_and_history() {
    let mut harness = right_pane_without_a_place();
    harness.press("Right");

    harness.press("~");
    harness.load(Side::Right, vec![file("a")]);

    assert_eq!(harness.view().right.path, HOME);
    assert_eq!(harness.names(Side::Right), ["..", "a"]);
    assert_eq!(harness.history(Side::Right), [HOME]);
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
                climb: true,
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
fn a_vanished_place_without_an_ancestor_is_left_for_home() {
    let mut harness = Harness::start("/mnt/usb", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.changed(Side::Left);
    harness.fail_load(Side::Left, "No such file or directory");
    harness.take_effects();

    harness.fail_resolve(Side::Left);

    assert_eq!(
        harness.log(),
        ["/mnt/usb is not available; moved to /home/user"]
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
fn opening_a_special_file_logs_an_error() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![special("fifo", SpecialKind::Fifo)]);
    harness.press("Down");
    harness.take_effects();

    harness.press("Enter");

    assert_eq!(
        harness.log(),
        [format!(
            "cannot open {}: not a regular file",
            PathBuf::from("/home/user").join("fifo").display()
        )]
    );
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn entering_a_special_file_as_an_archive_logs_an_error() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![special("sock", SpecialKind::Socket)]);
    harness.press("Down");
    harness.take_effects();

    harness.press("Shift+U");

    assert_eq!(
        harness.log(),
        [format!(
            "cannot open {}: not a regular file",
            PathBuf::from("/home/user").join("sock").display()
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
fn windows_only_keys_are_unbound_on_other_systems() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("L");
    harness.press("Ctrl+Left");
    harness.press("\\");

    assert_eq!(harness.log(), Vec::<String>::new());
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn drive_commands_bound_by_the_configuration_log_on_other_systems() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.config("[keys.filelist]\n\"L\" = \"drives\"\n\"Ctrl+Left\" = \"drive_prev\"\n");
    harness.take_effects();

    harness.press("L");
    harness.press("Ctrl+Left");

    assert_eq!(
        harness.log()[1..],
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
            climb: true,
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
fn size_format_applies_to_the_rows_and_the_marked_total_on_reload() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(
        Side::Left,
        vec![
            Entry {
                size: 1_234_567,
                ..file("a")
            },
            Entry {
                size: 512,
                ..file("b")
            },
        ],
    );
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    assert_eq!(harness.view().left.rows[1].size, "1234567");
    assert!(
        harness
            .view()
            .left
            .info
            .contains("1 marked (1234567 bytes)")
    );

    harness.config("[appearance]\nsize_format = \"comma\"\n");
    assert_eq!(harness.view().style.size_sample, "000,000,000,000");
    assert_eq!(harness.view().left.rows[1].size, "1,234,567");
    assert_eq!(harness.view().left.rows[2].size, "512");
    assert!(
        harness
            .view()
            .left
            .info
            .contains("1 marked (1,234,567 bytes)")
    );

    harness.config("[appearance]\nsize_format = \"human\"\n");
    assert_eq!(harness.view().style.size_sample, "0000.0K");
    assert_eq!(harness.view().left.rows[1].size, "1.2M");
    assert_eq!(harness.view().left.rows[2].size, "512");
    assert!(harness.view().left.info.contains("1 marked (1.2M)"));

    harness.config("[appearance]\nsize_format = \"bogus\"\n");
    assert_eq!(harness.view().left.rows[1].size, "1234567");
    assert!(
        harness
            .log()
            .iter()
            .any(|line| line.contains("appearance.size_format"))
    );
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
            RowKind::File,
            RowKind::Dir,
            RowKind::Link,
            RowKind::File,
            RowKind::Broken
        ]
    );
}

#[test]
fn executables_and_special_files_carry_their_own_kinds() {
    let mut harness = Harness::start("/home/user", "/tmp");

    harness.load(
        Side::Left,
        vec![
            Entry {
                kind: EntryKind::File { executable: true },
                ..file("run")
            },
            special("fifo", SpecialKind::Fifo),
            special("sock", SpecialKind::Socket),
            special("disk", SpecialKind::BlockDevice),
            special("tty", SpecialKind::CharDevice),
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
            RowKind::File,
            RowKind::Fifo,
            RowKind::Fifo,
            RowKind::Exec,
            RowKind::Socket,
            RowKind::Fifo
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
fn start_prefers_the_saved_display_state_and_falls_back_to_initial() {
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
fn start_without_a_saved_history_position_continues_from_the_newest_place() {
    let state = StateFile {
        left: PaneState {
            history: vec![PathBuf::from("/a"), PathBuf::from("/b")],
            history_pos: None,
            ..PaneState::default()
        },
        ..StateFile::default()
    };
    let mut harness = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Text(String::new()),
        Ok(Some(state)),
    ));
    harness.resolve(Side::Left, "/home/user", None);
    harness.load(Side::Left, vec![]);

    assert_eq!(harness.history(Side::Left), ["/a", "/b", "/home/user"]);
    assert_eq!(harness.app().pane(Side::Left).history().pos(), 2);
}

#[test]
fn start_without_a_state_file_uses_initial() {
    let harness = Harness::start_with_config(
        "/home/user",
        "/tmp",
        "[initial]\nsort = \"extension\"\ndescending = true\nshow_hidden = true\n",
    );

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
fn start_logs_a_created_configuration() {
    let harness = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Created,
        Ok(None),
    ));

    assert_eq!(harness.log(), [format!("created {CONFIG_PATH}")]);
}

#[test]
fn start_logs_an_unreadable_configuration_and_state_file() {
    let harness = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Unreadable(IoError::new("permission denied")),
        Err(StateFile::from_toml("[left\n").unwrap_err()),
    ));

    let log = harness.log();
    assert_eq!(log.len(), 2);
    assert_eq!(
        log[0],
        format!("cannot read {CONFIG_PATH}: permission denied")
    );
    assert!(log[1].starts_with("cannot read the state file: "));
}

#[test]
fn start_logs_a_broken_configuration_item_and_keeps_its_default() {
    let harness = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Text("[general]\nmkdir_then_enter =\n".to_owned()),
        Ok(None),
    ));

    let log = harness.log();
    assert_eq!(log.len(), 1);
    assert!(log[0].starts_with(&format!("config: {CONFIG_PATH}:2: ")));
    assert!(harness.app().config().general.mkdir_then_enter);
}

#[test]
fn start_logs_an_unknown_configuration_item() {
    let harness = Harness::start_from(start_msg_with(
        Os::Linux,
        Some("/home/user"),
        Some("/tmp"),
        ConfigSource::Text("[general]\nfoo = 1\n".to_owned()),
        Ok(None),
    ));

    assert_eq!(harness.log(), ["config: unknown item general.foo"]);
}

#[test]
fn reloading_the_configuration_changes_keys_and_style_but_not_initial() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.load(Side::Left, vec![file("a")]);
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.config(
        "[appearance]\ntheme = \"dark\"\nbogus = 1\n[appearance.font]\nsize = 16\n[initial]\nshow_hidden = true\n[keys.filelist]\n\"Q\" = \"nop\"\n\"F2\" = \"go_root\"\n",
    );

    assert_eq!(
        harness.log(),
        ["config: unknown item appearance.bogus", "config reloaded",]
    );
    let style = harness.view().style;
    assert_eq!(style.theme, Theme::Dark);
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
fn default_fonts_are_monospaced_except_in_the_popups() {
    let harness = Harness::start_on(Os::MacOs, "/Users/user", "/tmp");

    let fonts = harness.view().style.fonts.clone();
    assert_eq!(fonts.path.family, "Menlo");
    assert_eq!(fonts.filelist.family, "Menlo");
    assert_eq!(fonts.info.family, "Menlo");
    assert_eq!(fonts.log.family, "Menlo");
    assert_eq!(fonts.status.family, "Menlo");
    assert_eq!(fonts.preview.family, "Menlo");
    assert_eq!(fonts.popup.family, "");
}

#[test]
fn a_default_family_in_the_configuration_applies_to_every_area() {
    let mut harness = Harness::start("/home/user", "/tmp");

    harness.config(
        "[appearance.font]\nfamily = \"A\"\n[appearance.font.log]\nfamily = \"B\"\n[appearance.font.popup]\nfamily = \"C\"\n",
    );

    let fonts = harness.view().style.fonts.clone();
    assert_eq!(fonts.path.family, "A");
    assert_eq!(fonts.filelist.family, "A");
    assert_eq!(fonts.info.family, "A");
    assert_eq!(fonts.log.family, "B");
    assert_eq!(fonts.status.family, "A");
    assert_eq!(fonts.preview.family, "A");
    assert_eq!(fonts.popup.family, "C");
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
fn config_watch_failure_is_logged() {
    let mut harness = Harness::start("/home/user", "/tmp");

    harness.send(Msg::Io(IoEvent::ConfigWatchFailed {
        error: IoError::new("too many watches"),
    }));

    assert_eq!(
        harness.log(),
        [format!("cannot watch {CONFIG_PATH}: too many watches")]
    );
}

#[test]
fn quit_saves_the_state() {
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
}

#[test]
fn shift_q_quits_without_saving_the_state() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

    harness.press("Shift+Q");

    assert_eq!(harness.take_effects(), vec![Effect::Quit]);
}

#[test]
fn closing_the_window_saves_the_state_and_quits() {
    let mut harness = Harness::start("/home/user", "/tmp");
    harness.take_effects();

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

    harness.press("F1");
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
    let mut harness = Harness::start_with_config(
        "/home/user",
        "/tmp",
        "[general]\neditor = \"code -w {paths}\"\n[os.macos.general]\neditor = \"open -t {paths}\"\n",
    );
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
}

#[test]
fn the_default_editor_on_macos_is_open_t() {
    let harness = Harness::start_on(Os::MacOs, "/home/user", "/tmp");

    assert_eq!(
        harness.app().config().general.editor,
        CommandLine::parse("open -t {paths}").unwrap()
    );
}

fn right_pane_without_a_place() -> Harness {
    let mut harness = Harness::start_unresolved(Os::Linux, "/home/user", "/tmp");
    harness.resolve(Side::Left, "/home/user", None);
    harness.load(Side::Left, vec![]);
    harness.press("Esc");
    harness.take_effects();
    harness
}

fn timestamp() -> Timestamp {
    Timestamp {
        at: std::time::SystemTime::UNIX_EPOCH,
        local: LocalDateTime {
            year: 2026,
            month: 1,
            day: 2,
            hour: 3,
            minute: 4,
            second: 5,
        },
    }
}
