use std::ffi::OsString;
use std::path::PathBuf;

use crate::app::{Change, ConfigSource, Effect, Msg, Os, Side};
use crate::operation::{OperationEvent, OperationKind, OperationSpec, Outcome, Reply, Sources};
use crate::popup::Popup;
use crate::test_support::{Harness, dir, file, start_msg_with};
use crate::view::ItemMark;

const LEFT: &str = "/home/user";
const RIGHT: &str = "/tmp";

#[test]
fn copy_starts_an_operation_on_the_marked_entries_in_display_order() {
    let mut harness = three_files();
    harness.press("Down");
    harness.press("Space");
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("C");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::StartOperation(OperationSpec {
            kind: OperationKind::Copy,
            sources: Sources::Paths(vec![path(LEFT, "a"), path(LEFT, "c")]),
            dest: Some(PathBuf::from(RIGHT)),
            output: None,
        })]
    );
    assert_eq!(harness.marks(Side::Left), ["a", "c"]);
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Copying");
    assert!(popup.progress);
}

#[test]
fn move_without_marks_takes_the_cursor_entry() {
    let mut harness = three_files();
    harness.press("Down");
    harness.press("Down");
    harness.take_effects();

    harness.press("M");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::StartOperation(OperationSpec {
            kind: OperationKind::Move,
            sources: Sources::Paths(vec![path(LEFT, "b")]),
            dest: Some(PathBuf::from(RIGHT)),
            output: None,
        })]
    );
    assert_eq!(harness.view().popup.unwrap().title, "Moving");
}

#[test]
fn copy_on_the_parent_row_without_marks_does_nothing() {
    let mut harness = three_files();
    harness.take_effects();

    harness.press("C");

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.log(), Vec::<String>::new());
    assert_eq!(harness.view().popup, None);
}

#[test]
fn copy_without_a_location_on_the_other_side_is_logged() {
    let mut harness = Harness::start_both_pending(Os::Linux, LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a")]);
    harness.fail_load(Side::Right, "gone");
    harness.press("Down");
    harness.take_effects();

    harness.press("C");

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("cannot copy: the other pane has no location")
    );
}

#[test]
fn an_operation_blocks_everything_but_cancel_and_new_window() {
    let mut harness = copying();

    harness.press("Q");
    harness.press("Enter");
    harness.press("Down");
    harness.click(Side::Left, 1);
    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.cursor_name(Side::Left), "c");

    harness.press("Alt+Z");
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::SpawnNewWindow(_)]
    ));

    harness.press("Esc");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::OperationReply(Reply::Cancel)]
    );
    assert_eq!(harness.view().popup.unwrap().title, "Cancelling");

    harness.press("Esc");
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn progress_is_shown_in_the_popup() {
    let mut harness = copying();

    harness.operation(OperationEvent::Progress {
        current: path(LEFT, "b"),
        done: 2,
    });

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Copying");
    assert_eq!(popup.message, Some(format!("2 done: {}", shown(LEFT, "b"))));
    assert!(popup.progress);
}

#[test]
fn finished_items_lose_their_marks_and_skipped_ones_keep_them() {
    let mut harness = copying();

    harness.operation(OperationEvent::ItemDone { index: 0 });
    assert_eq!(harness.marks(Side::Left), ["c"]);

    harness.finish(Outcome::Completed {
        done: 1,
        skipped: 1,
    });

    assert_eq!(harness.marks(Side::Left), ["c"]);
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("copied 1 item (1 skipped)")
    );
    assert_eq!(harness.view().popup, None);
    assert!(matches!(
        harness.take_effects().as_slice(),
        [
            Effect::ReadDir {
                side: Side::Left,
                location: left,
                ..
            },
            Effect::ReadDir {
                side: Side::Right,
                location: right,
                ..
            },
        ] if left.base == *LEFT && right.base == *RIGHT
    ));
    harness.press("Q");
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::SaveState(_), Effect::Quit]
    ));
}

#[test]
fn a_failure_is_logged_with_the_path() {
    let mut harness = copying();

    harness.finish(Outcome::Failed {
        path: path(LEFT, "a"),
        error: crate::error::IoError::new("permission denied"),
        done: 0,
    });

    assert_eq!(
        harness.log().last().cloned(),
        Some(format!(
            "cannot copy {}: permission denied",
            shown(LEFT, "a")
        ))
    );
}

#[test]
fn a_conflict_opens_the_choices_and_letters_answer_them() {
    let mut harness = copying();

    harness.operation(OperationEvent::Conflict {
        source: path(LEFT, "a"),
        dest: path(RIGHT, "a"),
    });

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "File exists");
    assert_eq!(popup.message, Some(shown(RIGHT, "a")));
    assert_eq!(
        harness.popup_labels(),
        ["Overwrite", "Skip", "Rename", "Apply to all", "Abort"]
    );
    assert!(!popup.progress);

    harness.press("O");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::OperationReply(Reply::Overwrite { all: false })]
    );
    assert_eq!(harness.view().popup.unwrap().title, "Copying");

    harness.operation(OperationEvent::Conflict {
        source: path(LEFT, "c"),
        dest: path(RIGHT, "c"),
    });
    harness.press("A");
    assert_eq!(
        harness.view().popup.unwrap().items[3].mark,
        ItemMark::Check(true)
    );
    harness.press("s");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::OperationReply(Reply::Skip { all: true })]
    );
}

#[test]
fn a_conflict_is_answered_from_the_cursor_too() {
    let mut harness = conflicting();

    harness.press("Down");
    harness.press("Down");
    harness.press("Down");
    harness.press("Space");
    assert_eq!(
        harness.view().popup.unwrap().items[3].mark,
        ItemMark::Check(true)
    );
    harness.press("Enter");
    assert_eq!(
        harness.view().popup.unwrap().items[3].mark,
        ItemMark::Check(false)
    );
    harness.press("Up");
    harness.press("Up");
    harness.press("Enter");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::OperationReply(Reply::Skip { all: false })]
    );
}

#[test]
fn abort_cancels_the_operation() {
    let mut harness = conflicting();

    harness.press("Esc");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::OperationReply(Reply::Cancel)]
    );
    assert_eq!(harness.view().popup.unwrap().title, "Cancelling");
}

#[test]
fn rename_in_a_conflict_asks_for_a_name() {
    let mut harness = copying();
    harness.operation(OperationEvent::Conflict {
        source: path(LEFT, "a"),
        dest: path(RIGHT, "a.tar.gz"),
    });
    harness.press("R");
    assert_eq!(harness.view().popup.unwrap().fields[0].select, Some((0, 5)));
    harness.press("Esc");
    harness.press("S");
    harness.take_effects();
    harness.operation(OperationEvent::Conflict {
        source: path(LEFT, "a"),
        dest: path(RIGHT, "a"),
    });
    harness.press("A");

    harness.press("R");

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Rename");
    assert_eq!(
        popup.fields.first().map(|field| field.text.as_str()),
        Some("a")
    );
    assert_eq!(popup.fields[0].select, None);

    harness.press("Esc");
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "File exists");
    assert_eq!(popup.items[3].mark, ItemMark::Check(true));

    harness.press("R");
    harness.edit("a");
    harness.press("Enter");
    assert_eq!(harness.view().popup.unwrap().title, "File exists");
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("R");
    harness.edit("a (copy)");
    harness.press("Enter");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::OperationReply(Reply::Rename(OsString::from(
            "a (copy)"
        )))]
    );
    assert_eq!(harness.view().popup.unwrap().title, "Copying");
}

#[test]
fn closing_the_window_during_an_operation_asks_first() {
    let mut harness = conflicting();

    harness.send(Msg::CloseRequested);

    assert_eq!(harness.take_effects(), vec![]);
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Quit");
    assert_eq!(
        popup.message.as_deref(),
        Some("An operation is in progress")
    );
    assert_eq!(
        harness.popup_labels(),
        ["Continue the operation", "Cancel the operation and quit"]
    );
    assert_eq!(popup.cursor, Some(0));

    harness.press("Enter");
    assert_eq!(harness.view().popup.unwrap().title, "File exists");
    assert_eq!(harness.take_effects(), vec![]);

    harness.send(Msg::CloseRequested);
    harness.send(Msg::CloseRequested);
    harness.press("Esc");
    assert_eq!(harness.view().popup.unwrap().title, "File exists");

    harness.send(Msg::CloseRequested);
    harness.press("Down");
    harness.press("Enter");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::OperationReply(Reply::Cancel)]
    );
    assert_eq!(harness.view().popup.unwrap().title, "Cancelling");

    harness.finish(Outcome::Cancelled { done: 1 });
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("copy cancelled after 1 item")
    );
    let effects = harness.take_effects();
    assert!(matches!(
        effects.as_slice(),
        [
            Effect::ReadDir { .. },
            Effect::ReadDir { .. },
            Effect::SaveState(_),
            Effect::Quit
        ]
    ));
}

#[test]
fn delete_asks_once_and_names_a_single_target() {
    let mut harness = three_files();
    harness.press("Down");
    harness.take_effects();

    harness.press("D");

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Delete");
    assert_eq!(popup.message.as_deref(), Some("Delete \"a\"?"));
    assert_eq!(harness.popup_labels(), ["Delete", "Cancel"]);
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("N");
    assert_eq!(harness.view().popup, None);
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("D");
    harness.press("Esc");
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("D");
    harness.press("Y");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::StartOperation(OperationSpec {
            kind: OperationKind::Delete,
            sources: Sources::Paths(vec![path(LEFT, "a")]),
            dest: None,
            output: None,
        })]
    );
    assert_eq!(harness.view().popup.unwrap().title, "Deleting");
}

#[test]
fn delete_counts_several_targets_and_enter_confirms() {
    let mut harness = three_files();
    harness.press("Down");
    harness.press("Space");
    harness.press("Space");
    harness.take_effects();

    harness.press("D");
    assert_eq!(
        harness.view().popup.unwrap().message.as_deref(),
        Some("Delete 2 items?")
    );
    harness.press("Enter");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::StartOperation(OperationSpec {
            kind: OperationKind::Delete,
            sources: Sources::Paths(vec![path(LEFT, "a"), path(LEFT, "b")]),
            dest: None,
            output: None,
        })]
    );

    harness.finish(Outcome::Completed {
        done: 2,
        skipped: 0,
    });
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("deleted 2 items")
    );
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::ReadDir {
            side: Side::Left,
            ..
        }]
    ));
}

#[test]
fn trash_starts_at_once() {
    let mut harness = three_files();
    harness.press("Down");
    harness.take_effects();

    harness.press("Shift+D");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::StartOperation(OperationSpec {
            kind: OperationKind::Trash,
            sources: Sources::Paths(vec![path(LEFT, "a")]),
            dest: None,
            output: None,
        })]
    );
    assert_eq!(harness.view().popup.unwrap().title, "Moving to trash");

    harness.finish(Outcome::Completed {
        done: 1,
        skipped: 0,
    });
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("moved 1 item to the trash")
    );
}

#[test]
fn rename_opens_with_the_name_and_selects_everything_but_the_extension() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(
        Side::Left,
        vec![file("a.tar.gz"), file(".bashrc"), dir("conf.d")],
    );
    harness.press("Down");
    harness.take_effects();
    assert_eq!(harness.cursor_name(Side::Left), "conf.d");

    harness.press("R");
    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Rename");
    assert_eq!(
        popup.fields.first().map(|field| field.text.as_str()),
        Some("conf.d")
    );
    assert_eq!(popup.fields[0].select, None);
    harness.press("Esc");

    harness.press("Down");
    harness.press("R");
    let popup = harness.view().popup.unwrap();
    assert_eq!(
        popup.fields.first().map(|field| field.text.as_str()),
        Some(".bashrc")
    );
    assert_eq!(popup.fields[0].select, None);
    harness.press("Esc");

    harness.press("Down");
    harness.press("R");
    let popup = harness.view().popup.unwrap();
    assert_eq!(
        popup.fields.first().map(|field| field.text.as_str()),
        Some("a.tar.gz")
    );
    assert_eq!(popup.fields[0].select, Some((0, 5)));

    harness.edit("a.tar.gz");
    harness.press("Enter");
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("R");
    harness.edit("");
    harness.press("Enter");
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("R");
    harness.edit("b.txt");
    harness.press("Enter");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Rename {
            from: path(LEFT, "a.tar.gz"),
            to: path(LEFT, "b.txt"),
        }]
    );
    assert_eq!(harness.view().popup, None);
}

#[test]
fn rename_does_nothing_with_marks_or_on_the_parent_row() {
    let mut harness = three_files();
    harness.take_effects();

    harness.press("R");
    assert_eq!(harness.view().popup, None);

    harness.press("Down");
    harness.press("Space");
    harness.press("R");
    assert_eq!(harness.view().popup, None);
    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn a_rename_puts_the_cursor_on_the_new_name() {
    let mut harness = three_files();
    harness.press("Down");
    harness.take_effects();

    harness.changed_result(
        Change::Renamed {
            from: path(LEFT, "a"),
            to: path(LEFT, "z"),
        },
        None,
    );

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::ReadDir {
            side: Side::Left,
            ..
        }]
    ));
    harness.load(Side::Left, vec![file("b"), file("c"), file("z")]);
    assert_eq!(harness.cursor_name(Side::Left), "z");
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("renamed a to z")
    );

    harness.changed_result(
        Change::Renamed {
            from: path(LEFT, "z"),
            to: path(LEFT, "a"),
        },
        Some("busy"),
    );
    assert_eq!(
        harness.log().last().cloned(),
        Some(format!("cannot rename {}: busy", shown(LEFT, "z")))
    );
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn a_change_reloads_every_pane_showing_the_directory() {
    let mut harness = Harness::start(LEFT, LEFT);
    harness.take_effects();

    harness.changed_result(Change::FileCreated(path(LEFT, "n")), None);

    assert!(matches!(
        harness.take_effects().as_slice(),
        [
            Effect::ReadDir {
                side: Side::Left,
                ..
            },
            Effect::ReadDir {
                side: Side::Right,
                ..
            }
        ]
    ));
    harness.load(Side::Left, vec![file("n")]);
    harness.load(Side::Right, vec![file("n")]);
    assert_eq!(harness.cursor_name(Side::Left), "n");
    assert_eq!(harness.cursor_name(Side::Right), "..");
}

#[test]
fn mkdir_creates_nested_directories_and_enters_the_new_one() {
    let mut harness = three_files();
    harness.take_effects();

    harness.press("K");
    assert_eq!(harness.view().popup.unwrap().title, "Make directory");
    harness.edit("a/b/c");
    harness.press("Enter");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::Mkdir(PathBuf::from(LEFT).join("a/b/c"))]
    );

    harness.changed_result(Change::DirCreated(PathBuf::from(LEFT).join("a/b/c")), None);
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Watch { .. }, Effect::ReadDir { side: Side::Left, location, .. }]
            if location.base == PathBuf::from(LEFT).join("a/b/c")
    ));
}

#[test]
fn mkdir_without_entering_puts_the_cursor_on_the_first_component() {
    let mut harness = Harness::start_from(start_msg_with(
        Os::Linux,
        Some(LEFT),
        Some(RIGHT),
        ConfigSource::Text("[general]\nmkdir_then_enter = false\n".to_owned()),
        Ok(None),
    ));
    harness.resolve(Side::Left, LEFT, None);
    harness.resolve(Side::Right, RIGHT, None);
    harness.load(Side::Left, vec![file("x")]);
    harness.load(Side::Right, vec![]);
    harness.take_effects();

    harness.press("K");
    harness.edit("");
    harness.press("Enter");
    assert_eq!(harness.take_effects(), vec![]);

    harness.changed_result(Change::DirCreated(PathBuf::from(LEFT).join("a/b")), None);
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::ReadDir {
            side: Side::Left,
            ..
        }]
    ));
    harness.load(Side::Left, vec![dir("a"), file("x")]);
    assert_eq!(harness.cursor_name(Side::Left), "a");
    assert_eq!(
        harness.log().last().cloned(),
        Some(format!(
            "created {}",
            PathBuf::from(LEFT).join("a/b").display()
        ))
    );

    harness.changed_result(Change::DirCreated(path(LEFT, "x")), Some("already exists"));
    assert_eq!(
        harness.log().last().cloned(),
        Some(format!(
            "cannot create {}: already exists",
            shown(LEFT, "x")
        ))
    );
}

#[test]
fn new_file_creates_an_empty_file_and_puts_the_cursor_on_it() {
    let mut harness = three_files();
    harness.take_effects();

    harness.press("B");
    harness.edit("new_file");
    harness.press("Enter");
    assert_eq!(harness.view().popup.unwrap().title, "New file");
    harness.edit("n.txt");
    harness.press("Enter");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::CreateFile(path(LEFT, "n.txt"))]
    );

    harness.changed_result(Change::FileCreated(path(LEFT, "n.txt")), None);
    harness.take_effects();
    harness.load(Side::Left, vec![file("a"), file("n.txt")]);
    assert_eq!(harness.cursor_name(Side::Left), "n.txt");
    assert_eq!(
        harness.log().last().cloned(),
        Some(format!("created {}", shown(LEFT, "n.txt")))
    );
}

#[test]
fn new_text_creates_the_file_and_then_opens_it_in_the_editor() {
    let mut harness = three_files();
    harness.take_effects();

    harness.press("Shift+E");
    assert_eq!(harness.view().popup.unwrap().title, "New text");
    harness.edit("notes.txt");
    harness.press("Enter");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::CreateFile(path(LEFT, "notes.txt"))]
    );

    harness.changed_result(Change::FileCreated(path(LEFT, "notes.txt")), None);
    assert!(matches!(
        harness.take_effects().as_slice(),
        [
            Effect::Spawn { program, args, .. },
            Effect::ReadDir { side: Side::Left, .. }
        ] if program == "xdg-open" && *args == [path(LEFT, "notes.txt").into_os_string()]
    ));

    // A file made some other way is not opened.
    harness.load(Side::Left, vec![]);
    harness.changed_result(Change::FileCreated(path(LEFT, "other.txt")), None);
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::ReadDir { .. }]
    ));
}

#[test]
fn new_text_on_an_existing_name_only_logs() {
    let mut harness = three_files();
    harness.take_effects();

    harness.press("Shift+E");
    harness.edit("a");
    harness.press("Enter");
    harness.take_effects();
    harness.changed_result(Change::FileCreated(path(LEFT, "a")), Some("already exists"));

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(
        harness.log().last().cloned(),
        Some(format!(
            "cannot create {}: already exists",
            shown(LEFT, "a")
        ))
    );
    harness.changed_result(Change::FileCreated(path(LEFT, "a")), None);
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::ReadDir { .. }]
    ));
}

#[test]
fn edit_passes_every_target_to_the_editor() {
    let mut harness = three_files();
    harness.take_effects();

    harness.press("E");
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("Down");
    harness.press("Space");
    harness.press("Space");
    harness.press("E");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Spawn {
            program: "xdg-open".to_owned(),
            args: vec![
                path(LEFT, "a").into_os_string(),
                path(LEFT, "b").into_os_string()
            ],
            cwd: PathBuf::from(LEFT),
        }]
    );
}

#[test]
fn open_with_os_opens_the_cursor_entry_only() {
    let mut harness = three_files();
    harness.press("Down");
    harness.press("Space");
    harness.press("Down");
    harness.take_effects();

    harness.press("Ctrl+Enter");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::OpenWithOs(path(LEFT, "c"))]
    );
    assert_eq!(harness.cursor_name(Side::Left), "c");

    harness.press("Home");
    harness.press("Ctrl+Enter");
    assert_eq!(harness.take_effects(), vec![]);

    harness.send(Msg::Io(crate::app::IoEvent::Opened {
        path: path(LEFT, "c"),
        error: Some(crate::error::IoError::new("no application")),
    }));
    assert_eq!(
        harness.log().last().cloned(),
        Some(format!("cannot open {}: no application", shown(LEFT, "c")))
    );
}

#[test]
fn copy_path_and_copy_name_join_the_targets_with_newlines() {
    let mut harness = three_files();
    harness.press("Down");
    harness.press("Space");
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("B");
    harness.edit("copy_path");
    harness.press("Enter");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Clipboard(format!(
            "{}\n{}",
            shown(LEFT, "a"),
            shown(LEFT, "c")
        ))]
    );

    harness.press("B");
    harness.edit("copy_name");
    harness.press("Enter");
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Clipboard("a\nc".to_owned())]
    );

    harness.send(Msg::Io(crate::app::IoEvent::ClipboardSet {
        error: Some(crate::error::IoError::new("no clipboard")),
    }));
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("cannot copy to the clipboard: no clipboard")
    );
}

#[test]
fn copy_path_on_the_parent_row_does_nothing() {
    let mut harness = three_files();
    harness.take_effects();

    harness.press("B");
    harness.edit("copy_path");
    harness.press("Enter");

    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn shell_menu_is_windows_only() {
    let mut harness = three_files();
    harness.press("\\");
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("shell_menu is available only on Windows")
    );

    let mut windows = Harness::start_on(Os::Windows, "C:\\", "D:\\");
    windows.press("\\");
    assert_eq!(windows.log(), Vec::<String>::new());
}

fn three_files() -> Harness {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a"), file("b"), file("c")]);
    harness
}

/// A copy of `a` and `c` is running.
fn copying() -> Harness {
    let mut harness = three_files();
    harness.press("Down");
    harness.press("Space");
    harness.press("Down");
    harness.press("Space");
    harness.press("C");
    harness.take_effects();
    assert!(harness.popup().is_none());
    harness
}

/// A copy is waiting for the answer to a conflict on `a`.
fn conflicting() -> Harness {
    let mut harness = copying();
    harness.operation(OperationEvent::Conflict {
        source: path(LEFT, "a"),
        dest: path(RIGHT, "a"),
    });
    assert!(matches!(harness.popup(), Some(Popup::Conflict(_))));
    harness
}

fn path(dir: &str, name: &str) -> PathBuf {
    PathBuf::from(dir).join(name)
}

// The path as the log shows it, with the separator of the OS running the test.
fn shown(dir: &str, name: &str) -> String {
    path(dir, name).display().to_string()
}
