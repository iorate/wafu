use std::path::PathBuf;

use crate::app::{Effect, MouseInput, Msg, Side};
use crate::entry::Entry;
use crate::error::IoError;
use crate::location::Location;
use crate::operation::{OperationEvent, OperationKind, OperationSpec, Outcome, Renames, Sources};
use crate::popup::Popup;
use crate::test_support::{Harness, LEFT, RIGHT, dir, file, path};
use crate::view::InputField;

#[test]
fn rename_replace_opens_with_two_empty_fields_and_the_hint() {
    let mut harness = marked(&["a", "b", "c"], &["a", "c"]);

    harness.press("Shift+R");

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Rename (replace)");
    assert_eq!(
        popup.fields,
        [
            InputField {
                label: "Find",
                ..InputField::new(String::new())
            },
            InputField {
                label: "Replace",
                ..InputField::new(String::new())
            },
        ]
    );
    assert_eq!(
        popup.note.as_deref(),
        Some("Replaces every occurrence, ignoring case")
    );
}

#[test]
fn rename_template_opens_with_the_name_placeholder_selected() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a.txt"), file("b.txt")]);
    harness.press("Down");

    harness.press("Ctrl+Shift+R");

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Rename (template)");
    assert_eq!(
        popup.fields,
        [InputField {
            select: Some((0, 6)),
            ..InputField::new("{stem}{ext}".to_owned())
        }]
    );
    assert_eq!(
        popup.note.as_deref(),
        Some("Placeholders: {stem} (name without extension), {ext}, {001} (counter from 001)")
    );
}

#[cfg(unix)]
#[test]
fn a_batch_rename_with_a_target_whose_name_is_not_utf8_is_refused() {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    use crate::test_support::shown;

    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(
        Side::Left,
        vec![
            file("a"),
            Entry {
                name: OsStr::from_bytes(b"b\xff").to_owned(),
                ..file("")
            },
        ],
    );
    harness.press("Down");
    harness.press("Space");
    harness.press("Space");
    assert_eq!(harness.marks(Side::Left).len(), 2);

    harness.press("Shift+R");

    assert_eq!(harness.popup(), None);
    assert_eq!(
        harness.log().last().cloned(),
        Some(format!(
            "Cannot rename {}: the name is not valid UTF-8",
            shown(LEFT, "b\u{fffd}")
        ))
    );
}

#[test]
fn a_batch_rename_on_the_parent_row_renames_the_directory_itself_whatever_its_listing_holds() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![dir("me")]);
    assert_eq!(harness.cursor_name(Side::Left), "..");

    harness.press("Ctrl+Shift+R");
    harness.edit("me");
    harness.take_effects();
    harness.press("Enter");

    assert_eq!(
        harness.start_operation(),
        Some(OperationSpec {
            kind: OperationKind::Rename,
            sources: Sources::Renames(Renames {
                dir: PathBuf::from("/home"),
                pairs: vec![("user".to_owned(), "me".to_owned())],
            }),
            dest: None,
            output: None,
        })
    );
}

#[test]
fn the_list_shows_only_the_targets_that_change_and_the_note_the_first_error() {
    let mut harness = marked(&["a1", "b1", "c"], &["a1", "b1"]);
    harness.press("Shift+R");

    harness.edit("a1");
    harness.edit_field(1, "c");

    assert_eq!(shown_items(&harness), ["a1 \u{2192} c"]);
    assert_eq!(harness.view().popup.unwrap().lines.unwrap().count, 1);
    assert_eq!(
        harness.view().popup.unwrap().note.as_deref(),
        Some("a1 \u{2192} c: already exists")
    );
}

#[test]
fn a_name_hidden_from_the_list_is_still_in_the_way() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(
        Side::Left,
        vec![
            file("a"),
            Entry {
                hidden: true,
                ..file(".x")
            },
        ],
    );
    harness.press("Down");
    harness.press("Ctrl+Shift+R");

    harness.edit(".x");

    assert_eq!(
        harness.view().popup.unwrap().note.as_deref(),
        Some("a \u{2192} .x: already exists")
    );
}

#[test]
fn the_note_follows_the_listing_as_it_is_read_again() {
    let mut harness = marked(&["a", "b"], &["a"]);
    harness.press("Ctrl+Shift+R");
    harness.edit("b");

    harness.load(Side::Left, vec![file("a")]);

    assert_eq!(
        harness.view().popup.unwrap().note.as_deref(),
        Some("Placeholders: {stem} (name without extension), {ext}, {001} (counter from 001)")
    );
}

#[test]
fn enter_does_nothing_without_a_change_or_with_an_error_and_esc_closes() {
    let mut harness = marked(&["a", "b"], &["a"]);
    harness.press("Ctrl+Shift+R");
    harness.take_effects();

    harness.press("Enter");
    assert!(matches!(harness.popup(), Some(Popup::Rename(_))));
    harness.edit("b");
    harness.press("Enter");
    assert!(matches!(harness.popup(), Some(Popup::Rename(_))));
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("Esc");
    assert_eq!(harness.popup(), None);
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn enter_renames_the_targets_that_change_in_display_order_and_unmarks_the_rest() {
    let mut harness = marked(&["a1", "b", "c1"], &["a1", "b", "c1"]);
    harness.press("Shift+R");
    harness.edit("1");
    harness.edit_field(1, "2");
    harness.take_effects();

    harness.press("Enter");

    assert_eq!(
        harness.start_operation(),
        Some(OperationSpec {
            kind: OperationKind::Rename,
            sources: Sources::Renames(Renames {
                dir: PathBuf::from(LEFT),
                pairs: vec![
                    ("a1".to_owned(), "a2".to_owned()),
                    ("c1".to_owned(), "c2".to_owned()),
                ],
            }),
            dest: None,
            output: None,
        })
    );
    assert_eq!(harness.marks(Side::Left), ["a1", "c1"]);
    assert_eq!(harness.view().popup.unwrap().title, "Renaming");
}

#[test]
fn a_renamed_item_loses_its_mark_by_its_old_name() {
    let mut harness = renaming();

    harness.operation(OperationEvent::ItemDone { index: 1 });

    assert_eq!(harness.marks(Side::Left), ["a1"]);
}

#[test]
fn a_finished_batch_rename_counts_the_items_renamed() {
    let mut harness = renaming();

    harness.finish(Outcome::Completed { skipped: 0 });

    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("Renamed 2 items")
    );
}

#[test]
fn a_batch_rename_of_one_item_logs_both_names() {
    let mut harness = marked(&["a1", "b"], &["a1", "b"]);
    harness.press("Shift+R");
    harness.edit("1");
    harness.edit_field(1, "2");
    harness.press("Enter");

    harness.finish(Outcome::Completed { skipped: 0 });

    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("Renamed a1 to a2")
    );
}

#[test]
fn a_broken_cycle_unmarks_its_items_and_names_the_one_left_under_a_temporary_name() {
    let mut harness = marked(&["1", "2", "3"], &["1", "2", "3"]);
    harness.press("Ctrl+Shift+R");
    harness.edit("{2}");
    harness.press("Enter");

    harness.operation(OperationEvent::CycleBroken {
        cycle: vec![0, 1],
        left: 0,
        temp: "wafu-rename-0.tmp".to_owned(),
    });
    harness.finish(Outcome::Failed {
        path: path(LEFT, "2"),
        error: IoError::new("busy"),
    });

    assert_eq!(harness.marks(Side::Left), ["3"]);
    assert_eq!(
        harness.log().last().cloned(),
        Some(format!(
            "Cannot rename {}: busy (1 left as wafu-rename-0.tmp)",
            path(LEFT, "2").display()
        ))
    );
}

#[test]
fn the_cursor_follows_its_item_to_the_new_name_even_when_the_rest_fails() {
    let mut harness = renaming();
    assert_eq!(harness.cursor_name(Side::Left), "c1");
    harness.operation(OperationEvent::ItemDone { index: 1 });

    harness.finish(Outcome::Failed {
        path: path(LEFT, "a1"),
        error: IoError::new("busy"),
    });
    harness.load(Side::Left, vec![file("a1"), file("b"), file("c2")]);

    assert_eq!(harness.cursor_name(Side::Left), "c2");
}

#[test]
fn the_cursor_follows_the_item_it_was_on_at_enter_through_reloads_in_a_chain() {
    let mut harness = marked(&["1", "2", "3"], &["1", "2", "3"]);
    harness.press("Up");
    assert_eq!(harness.cursor_name(Side::Left), "2");
    harness.press("Ctrl+Shift+R");
    harness.edit("{2}");
    harness.press("Enter");

    harness.operation(OperationEvent::ItemDone { index: 2 });
    harness.load(Side::Left, vec![file("1"), file("2"), file("4")]);
    harness.operation(OperationEvent::ItemDone { index: 1 });
    harness.load(Side::Left, vec![file("1"), file("3"), file("4")]);
    harness.operation(OperationEvent::ItemDone { index: 0 });
    harness.finish(Outcome::Completed { skipped: 0 });
    harness.load(Side::Left, vec![file("2"), file("3"), file("4")]);

    assert_eq!(harness.cursor_name(Side::Left), "3");
}

#[test]
fn the_cursor_stays_on_an_item_that_was_not_renamed() {
    let mut harness = marked(&["1", "2"], &["1", "2"]);
    harness.press("Up");
    assert_eq!(harness.cursor_name(Side::Left), "1");
    harness.press("Ctrl+Shift+R");
    harness.edit("{2}");
    harness.press("Enter");

    harness.finish(Outcome::Failed {
        path: path(LEFT, "2"),
        error: IoError::new("busy"),
    });
    harness.load(Side::Left, vec![file("1"), file("2")]);

    assert_eq!(harness.cursor_name(Side::Left), "1");
}

#[test]
fn a_pane_showing_a_renamed_directory_follows_it() {
    let shown_dir = path(LEFT, "d1");
    let mut harness = Harness::start(LEFT, shown_dir.to_str().unwrap());
    harness.load(Side::Left, vec![dir("d1")]);
    harness.press("Down");
    harness.press("Shift+R");
    harness.edit("1");
    harness.edit_field(1, "2");
    harness.press("Enter");
    harness.operation(OperationEvent::ItemDone { index: 0 });
    harness.take_effects();

    harness.finish(Outcome::Completed { skipped: 0 });

    let moved = Location::new(path(LEFT, "d2"));
    assert!(harness.take_effects().iter().any(|effect| matches!(
        effect,
        Effect::ReadDir { side: Side::Right, location, .. } if *location == moved
    )));
}

#[test]
fn alt_keys_scroll_the_list_by_items_pages_and_to_the_ends() {
    let mut harness = long_list(20);

    harness.press("Alt+Down");
    assert_eq!(top(&harness), 1);
    harness.press("Alt+Up");
    harness.press("Alt+Up");
    assert_eq!(top(&harness), 0);
    harness.press("Alt+PageDown");
    assert_eq!(top(&harness), 5);
    harness.press("Alt+PageUp");
    assert_eq!(top(&harness), 0);
    harness.press("Alt+End");
    assert_eq!(top(&harness), 15);
    harness.press("Alt+Home");
    assert_eq!(top(&harness), 0);
}

#[test]
fn arrows_without_alt_or_with_more_than_alt_leave_the_list_alone() {
    let mut harness = long_list(20);

    for key in ["Down", "PageDown", "Alt+Shift+Down", "Ctrl+Alt+Down"] {
        harness.press(key);
        assert_eq!(top(&harness), 0, "{key}");
    }
}

#[test]
fn the_wheel_scrolls_the_list() {
    let mut harness = long_list(20);

    harness.popup_wheel(3);

    assert_eq!(top(&harness), 3);
}

#[test]
fn the_scrollbar_puts_an_item_at_the_top_up_to_the_end() {
    let mut harness = long_list(20);

    harness.send(Msg::Mouse(MouseInput::PopupScrollTo { top: 7 }));
    assert_eq!(top(&harness), 7);
    harness.send(Msg::Mouse(MouseInput::PopupScrollTo { top: 100 }));
    assert_eq!(top(&harness), 15);
}

#[test]
fn the_top_item_stays_as_the_list_changes_and_comes_back_once_the_end_unwraps() {
    let mut harness = long_list(20);
    let tail: Vec<(usize, usize)> = (15..20).map(|index| (index, 2)).collect();
    harness.send(Msg::PopupLineRows {
        area: None,
        lines: tail,
    });
    harness.press("Alt+End");
    assert_eq!(top(&harness), 18);

    harness.edit("{1}");
    assert_eq!(top(&harness), 18);

    let unwrapped: Vec<(usize, usize)> = (15..20).map(|index| (index, 1)).collect();
    harness.send(Msg::PopupLineRows {
        area: None,
        lines: unwrapped,
    });
    assert_eq!(top(&harness), 15);
}

#[test]
fn a_list_that_shrinks_past_the_top_brings_it_back_to_its_last_item() {
    let mut harness = replacing_all(20);
    harness.press("Alt+End");
    assert_eq!(top(&harness), 15);

    harness.edit("f0");

    assert_eq!(top(&harness), 9);
}

#[test]
fn rows_measured_for_the_item_that_had_an_index_do_not_pull_the_top_back() {
    let mut harness = replacing_all(20);
    harness.press("Alt+End");
    // `f10` to `f19` take the indices of `f00` to `f09`.
    harness.edit("f1");
    assert_eq!(top(&harness), 9);

    harness.send(Msg::PopupLineRows {
        area: Some(5),
        lines: Vec::new(),
    });

    assert_eq!(top(&harness), 9);
}

#[test]
fn an_area_reported_without_a_rename_popup_does_not_change_the_page_of_another_list() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.press("?");
    harness.send(Msg::PopupRows { rows: 12 });

    harness.send(Msg::PopupLineRows {
        area: Some(3),
        lines: Vec::new(),
    });
    harness.press("PageDown");

    assert_eq!(harness.view().popup.unwrap().cursor, Some(12));
}

#[test]
fn line_breaks_in_names_show_as_symbols_in_the_list_and_the_note() {
    let mut harness = marked(&["a\r", "b"], &["a\r"]);

    harness.press("Ctrl+Shift+R");
    harness.edit("b");

    let popup = harness.view().popup.unwrap();
    assert_eq!(shown_items(&harness), ["a\u{21b5} \u{2192} b"]);
    assert_eq!(
        popup.note.as_deref(),
        Some("a\u{21b5} \u{2192} b: already exists")
    );
}

// The files of `names` in the left pane, with those of `marks` marked and
// the cursor on the last one.
fn marked(names: &[&str], marks: &[&str]) -> Harness {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, names.iter().map(|name| file(name)).collect());
    harness.press("Down");
    for name in names {
        harness.press(if marks.contains(name) {
            "Space"
        } else {
            "Down"
        });
    }
    assert_eq!(harness.marks(Side::Left), marks);
    harness
}

// A rename of `a1` to `a2` and `c1` to `c2` is running, with `b` marked too.
fn renaming() -> Harness {
    let mut harness = marked(&["a1", "b", "c1"], &["a1", "b", "c1"]);
    harness.press("Shift+R");
    harness.edit("1");
    harness.edit_field(1, "2");
    harness.press("Enter");
    harness.take_effects();
    harness
}

// A template popup on `count` files that renames them all, its list five rows
// high and every item measured at one row.
fn long_list(count: usize) -> Harness {
    let names: Vec<String> = (0..count).map(|index| format!("f{index:02}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut harness = marked(&names, &names);
    harness.press("Ctrl+Shift+R");
    harness.edit("x{stem}");
    harness.send(Msg::PopupLineRows {
        area: Some(5),
        lines: (0..count).map(|index| (index, 1)).collect(),
    });
    harness.take_effects();
    harness
}

// A replace popup on `count` files that renames them all, its list five rows
// high and every item measured at one row.
fn replacing_all(count: usize) -> Harness {
    let names: Vec<String> = (0..count).map(|index| format!("f{index:02}")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let mut harness = marked(&names, &names);
    harness.press("Shift+R");
    harness.edit("f");
    harness.edit_field(1, "g");
    harness.send(Msg::PopupLineRows {
        area: Some(5),
        lines: (0..count).map(|index| (index, 1)).collect(),
    });
    harness.take_effects();
    harness
}

fn shown_items(harness: &Harness) -> Vec<String> {
    lines(harness).into_iter().map(|line| line.text).collect()
}

fn top(harness: &Harness) -> usize {
    lines(harness)[0].index
}

fn lines(harness: &Harness) -> Vec<crate::view::PopupLine> {
    harness.view().popup.unwrap().lines.unwrap().shown
}
