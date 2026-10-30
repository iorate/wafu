use std::path::PathBuf;

use crate::app::{Effect, IoEvent, Msg, Os, Side};
use crate::entry::Directory;
use crate::location::{ArchiveFormat, ArchivePath, Location};
use crate::operation::{
    ArchiveEntries, ArchiveOutput, OperationEvent, OperationKind, OperationSpec, Outcome, Reply,
    Sources,
};
use crate::pane::RequestId;
use crate::test_support::{Harness, dir, file};

const LEFT: &str = "/home/user";
const RIGHT: &str = "/tmp";
const TEMP: &str = "/tmp/wafu/x1";

#[test]
fn enter_on_a_zip_watches_the_archive_and_shows_its_path() {
    let mut harness = outside();

    harness.press("Enter");

    let effects = harness.take_effects();
    assert_eq!(
        effects,
        vec![
            Effect::Watch {
                side: Side::Left,
                path: archive(),
            },
            Effect::ReadDir {
                side: Side::Left,
                location: root(),
                request: harness.latest_request(Side::Left),
            },
        ]
    );

    harness.load(Side::Left, contents());

    assert_eq!(harness.view().left.path, root().to_string());
    assert_eq!(harness.view().title, format!("wafu - {}", root()));
    assert_eq!(
        harness.names(Side::Left),
        ["..", "docs", "b.txt", "inner.zip", "secret.txt"]
    );
    assert_eq!(harness.history(Side::Left), [LEFT]);
}

#[test]
fn enter_as_archive_opens_any_file_but_not_a_directory() {
    let mut harness = outside();
    harness.press("Down");
    harness.take_effects();

    harness.press("Shift+U");

    let effects = harness.take_effects();
    assert!(matches!(
        effects.as_slice(),
        [Effect::Watch { .. }, Effect::ReadDir { location, .. }]
            if *location == Location::archive_root(path(LEFT, "note.txt"), ArchiveFormat::Zip)
    ));
    harness.press("Esc");
    harness.press("Up");
    harness.press("Up");
    harness.take_effects();

    harness.press("Shift+U");

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.cursor_name(Side::Left), "d");
}

#[test]
fn directories_inside_an_archive_descend_and_backspace_returns_with_the_cursor() {
    let mut harness = inside();
    harness.press("Down");

    harness.press("Enter");

    let effects = harness.take_effects();
    assert!(matches!(
        effects.as_slice(),
        [Effect::Watch { path, .. }, Effect::ReadDir { location, .. }]
            if *path == archive() && *location == root().child("docs")
    ));
    harness.load(Side::Left, vec![file("x")]);
    assert_eq!(harness.view().left.path, format!("{}/docs", root()));
    assert_eq!(harness.names(Side::Left), ["..", "x"]);
    harness.take_effects();

    harness.press("BS");
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Watch { .. }, Effect::ReadDir { location, .. }] if *location == root()
    ));
    harness.load(Side::Left, contents());

    assert_eq!(harness.cursor_name(Side::Left), "docs");
    assert_eq!(harness.history(Side::Left), [LEFT]);
}

#[test]
fn go_root_inside_an_archive_goes_to_the_archive_root_and_stays_there() {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Enter");
    harness.load(Side::Left, vec![dir("deep")]);
    harness.press("Down");
    harness.press("Enter");
    harness.load(Side::Left, vec![file("x")]);
    harness.take_effects();

    harness.press("/");

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Watch { path, .. }, Effect::ReadDir { location, .. }]
            if *path == archive() && *location == root()
    ));
    harness.load(Side::Left, contents());
    assert_eq!(harness.view().left.path, root().to_string());
    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert_eq!(harness.history(Side::Left), [LEFT]);
    harness.take_effects();

    harness.press("/");

    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn backspace_at_the_archive_root_leaves_with_the_cursor_on_the_archive() {
    let mut harness = inside();

    harness.press("BS");

    assert_eq!(
        harness.take_effects(),
        vec![
            Effect::Watch {
                side: Side::Left,
                path: PathBuf::from(LEFT),
            },
            Effect::ReadDir {
                side: Side::Left,
                location: Location::new(PathBuf::from(LEFT)),
                request: harness.latest_request(Side::Left),
            },
        ]
    );
    harness.load(Side::Left, vec![file("a.zip"), file("note.txt"), dir("d")]);

    assert_eq!(harness.cursor_name(Side::Left), "a.zip");
    assert_eq!(harness.view().left.path, LEFT);
}

#[test]
fn escape_while_entering_an_archive_stays_put() {
    let mut harness = outside();
    harness.press("Enter");
    harness.take_effects();

    harness.press("Esc");

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::CancelRead { .. }, Effect::Watch { path, .. }] if path.as_os_str() == LEFT
    ));
    assert_eq!(harness.view().left.path, LEFT);
    assert_eq!(harness.cursor_name(Side::Left), "a.zip");
}

#[test]
fn archives_inside_an_archive_cannot_be_entered() {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Down");
    harness.press("Down");
    assert_eq!(harness.cursor_name(Side::Left), "inner.zip");

    harness.press("Enter");
    harness.press("Shift+U");

    assert_eq!(harness.take_effects(), vec![]);
    let message = format!(
        "cannot open {}/inner.zip: nested archives are not supported",
        root()
    );
    assert_eq!(harness.log(), [message.clone(), message]);
}

#[test]
fn commands_that_change_things_are_refused_inside_an_archive() {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Down");

    for key in ["M", "D", "Shift+D", "R", "K", "Shift+E", "U", "P", "\\"] {
        harness.press(key);
    }

    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.popup().is_none());
    assert_eq!(
        harness.log(),
        [
            "move is not available inside an archive",
            "delete is not available inside an archive",
            "trash is not available inside an archive",
            "rename is not available inside an archive",
            "mkdir is not available inside an archive",
            "new_text is not available inside an archive",
            "extract is not available inside an archive",
            "compress is not available inside an archive",
        ]
    );
}

#[test]
fn right_click_inside_an_archive_on_windows_is_refused() {
    let mut harness = Harness::start_on(Os::Windows, r"C:\Users\me", r"D:\");
    harness.load(Side::Left, vec![file("a.zip")]);
    harness.press("Down");
    harness.enter_archive(Side::Left, contents());
    harness.take_effects();

    harness.right_click(Side::Left, 2);

    assert_eq!(harness.cursor_name(Side::Left), "b.txt");
    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(
        harness.log(),
        ["shell_menu is not available inside an archive"]
    );
}

#[test]
fn nothing_can_be_sent_into_an_archive_on_the_other_side() {
    let mut harness = inside();
    harness.press("Tab");
    harness.load(Side::Right, vec![file("r.txt")]);
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    for key in ["C", "M", "U", "P"] {
        harness.press(key);
    }

    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.popup().is_none());
    assert_eq!(
        harness.log(),
        [
            "cannot copy into an archive",
            "cannot move into an archive",
            "cannot extract into an archive",
            "cannot compress into an archive",
        ]
    );
}

#[test]
fn with_no_target_nothing_is_said_about_an_archive_on_the_other_side() {
    let mut harness = inside();
    harness.config("[general]\nrequire_marks = false\n");
    harness.press("Tab");
    harness.load(Side::Right, vec![file("r.txt")]);
    harness.take_effects();
    assert_eq!(harness.cursor_name(Side::Right), "..");
    let log = harness.log();

    for key in ["C", "M", "U"] {
        harness.press(key);
    }

    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.popup().is_none());
    assert_eq!(harness.log(), log);
}

#[test]
fn copy_inside_an_archive_extracts_the_marked_entries_to_the_other_side() {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Down");
    harness.press("Space");
    harness.press("Space");
    harness.take_effects();

    harness.press("C");

    assert_eq!(
        harness.start_operation(),
        Some(OperationSpec {
            kind: OperationKind::Copy,
            sources: Sources::Entries(entries(&["b.txt", "inner.zip"], None)),
            dest: Some(PathBuf::from(RIGHT)),
            output: None,
        })
    );
    assert_eq!(harness.view().popup.unwrap().title, "Copying");

    harness.operation(OperationEvent::ItemDone { index: 0 });
    assert_eq!(harness.marks(Side::Left), ["inner.zip"]);

    harness.finish(Outcome::Completed {
        done: 2,
        skipped: 0,
    });

    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("copied 2 items")
    );
    let effects = harness.take_effects();
    assert!(matches!(
        effects.as_slice(),
        [Effect::ReadDir { side: Side::Left, location, .. }, Effect::ReadDir { side: Side::Right, .. }]
            if *location == root()
    ));
}

#[test]
fn extract_unpacks_the_targets_into_the_other_side() {
    let mut harness = outside();
    harness.take_effects();

    harness.press("U");

    assert_eq!(
        harness.start_operation(),
        Some(OperationSpec {
            kind: OperationKind::Extract,
            sources: Sources::Paths(vec![archive()]),
            dest: Some(PathBuf::from(RIGHT)),
            output: None,
        })
    );
    assert_eq!(harness.view().popup.unwrap().title, "Extracting");

    harness.finish(Outcome::Completed {
        done: 5,
        skipped: 0,
    });

    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("extracted 5 items")
    );
}

#[test]
fn extract_and_compress_with_no_target_do_nothing() {
    let mut harness = outside();
    harness.press("Up");
    harness.press("Up");
    harness.take_effects();
    assert_eq!(harness.cursor_name(Side::Left), "..");

    harness.press("U");
    harness.press("P");

    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.popup().is_none());
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn compress_asks_for_the_name_and_a_password_in_one_popup() {
    let mut harness = outside();
    harness.press("Down");
    harness.take_effects();

    harness.press("P");

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Compress");
    assert_eq!(
        popup.message.as_deref(),
        Some("Leave the password empty for no encryption")
    );
    let labels: Vec<&str> = popup.fields.iter().map(|field| field.label).collect();
    assert_eq!(labels, ["Name", "Password"]);
    assert_eq!(harness.popup_fields(), ["note.txt.zip", ""]);
    assert_eq!(popup.fields[0].select, Some((0, 8)));
    assert!(!popup.fields[0].secret);
    assert!(popup.fields[1].secret);

    harness.press("Enter");

    assert_eq!(
        harness.start_operation(),
        Some(OperationSpec {
            kind: OperationKind::Compress,
            sources: Sources::Paths(vec![path(LEFT, "note.txt")]),
            dest: Some(PathBuf::from(RIGHT)),
            output: Some(ArchiveOutput {
                format: ArchiveFormat::Zip,
                name: "note.txt.zip".into(),
                password: None,
            }),
        })
    );
    assert_eq!(harness.view().popup.unwrap().title, "Compressing");

    harness.finish(Outcome::Completed {
        done: 1,
        skipped: 0,
    });

    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("compressed 1 item")
    );
}

#[test]
fn compress_takes_the_password_typed_in_the_second_field() {
    let mut harness = outside();
    harness.press("Down");
    harness.press("P");
    harness.edit("backup.zip");
    harness.edit_field(1, "hunter2");
    harness.take_effects();

    harness.press("Enter");

    let spec = harness.start_operation().unwrap();
    assert_eq!(
        spec.output,
        Some(ArchiveOutput {
            format: ArchiveFormat::Zip,
            name: "backup.zip".into(),
            password: Some("hunter2".to_owned()),
        })
    );
}

#[test]
fn compress_takes_the_directories_of_both_panes_when_the_popup_was_opened() {
    let mut harness = outside();
    harness.press("Down");
    harness.press("P");
    harness.vanish(Side::Left, "/home", vec![file("note.txt")]);
    harness.vanish(Side::Right, "/", vec![]);
    assert_eq!(harness.view().left.path, "/home");
    assert_eq!(harness.view().right.path, "/");

    harness.press("Enter");

    let spec = harness.start_operation().unwrap();
    assert_eq!(spec.sources, Sources::Paths(vec![path(LEFT, "note.txt")]));
    assert_eq!(spec.dest, Some(PathBuf::from(RIGHT)));
}

#[test]
fn compress_names_the_archive_after_the_directory_for_several_targets() {
    let mut harness = outside();
    harness.press("Space");
    harness.press("Space");
    harness.take_effects();

    harness.press("P");

    assert_eq!(harness.popup_fields(), ["user.zip", ""]);
    harness.press("Esc");
    assert!(harness.popup().is_none());
    assert_eq!(harness.take_effects(), vec![]);

    harness.press("Down");
    harness.press("Down");
    assert_eq!(harness.cursor_name(Side::Left), "d");
    harness.press("A");
    harness.press("P");

    assert_eq!(harness.popup_fields(), ["d.zip", ""]);
}

#[test]
fn compress_at_the_root_falls_back_to_a_fixed_name() {
    let mut harness = Harness::start("/", RIGHT);
    harness.load(Side::Left, vec![file("x"), file("y")]);
    harness.press("Space");
    harness.press("Space");

    harness.press("P");

    assert_eq!(harness.popup_fields(), ["archive.zip", ""]);
}

#[test]
fn compress_with_an_empty_name_just_closes() {
    let mut harness = outside();
    harness.press("P");
    harness.edit("");
    harness.take_effects();

    harness.press("Enter");

    assert!(harness.popup().is_none());
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn compress_without_a_location_on_the_other_side_is_logged() {
    let mut harness = Harness::start_both_pending(Os::Linux, LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a")]);
    harness.fail_load(Side::Right, "gone");
    harness.press("Down");

    harness.press("P");

    assert!(harness.popup().is_none());
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("cannot compress: the other pane has no location")
    );
}

#[test]
fn a_password_is_asked_once_and_remembered_inside_the_archive() {
    let mut harness = copying_inside();

    harness.operation(OperationEvent::NeedPassword {
        archive: archive(),
        retry: false,
    });

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Password");
    assert_eq!(popup.message.as_deref(), Some("a.zip"));
    assert!(popup.fields[0].secret);

    harness.press("Enter");
    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.view().popup.unwrap().title, "Password");

    harness.edit("pw");
    harness.press("Enter");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::OperationReply(Reply::Password("pw".to_owned()))]
    );
    assert_eq!(harness.view().popup.unwrap().title, "Copying");

    harness.finish(Outcome::Completed {
        done: 1,
        skipped: 0,
    });
    harness.take_effects();
    harness.press("C");

    let spec = harness.start_operation().unwrap();
    assert_eq!(
        spec.sources,
        Sources::Entries(entries(&["b.txt"], Some("pw")))
    );
}

#[test]
fn a_password_request_during_the_quit_confirmation_waits_behind_it() {
    let mut harness = copying_inside();
    harness.send(Msg::CloseRequested);

    harness.operation(OperationEvent::NeedPassword {
        archive: archive(),
        retry: false,
    });

    assert_eq!(harness.view().popup.unwrap().title, "Quit");
    harness.press("Enter");
    assert_eq!(harness.view().popup.unwrap().title, "Password");
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn a_wrong_password_is_asked_again_and_forgotten() {
    let mut harness = copying_inside();
    harness.operation(OperationEvent::NeedPassword {
        archive: archive(),
        retry: false,
    });
    harness.edit("bad");
    harness.press("Enter");
    harness.take_effects();

    harness.operation(OperationEvent::NeedPassword {
        archive: archive(),
        retry: true,
    });

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.message.as_deref(), Some("Wrong password for a.zip"));

    harness.press("Esc");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::OperationReply(Reply::Cancel)]
    );
    assert_eq!(harness.view().popup.unwrap().title, "Cancelling");

    harness.finish(Outcome::Cancelled { done: 0 });
    harness.take_effects();
    harness.press("C");

    let spec = harness.start_operation().unwrap();
    assert_eq!(spec.sources, Sources::Entries(entries(&["b.txt"], None)));
}

#[test]
fn passwords_typed_for_extract_are_not_remembered() {
    let mut harness = outside();
    harness.press("U");
    harness.take_effects();
    harness.operation(OperationEvent::NeedPassword {
        archive: archive(),
        retry: false,
    });
    harness.edit("pw");
    harness.press("Enter");
    harness.finish(Outcome::Completed {
        done: 1,
        skipped: 0,
    });
    harness.enter_archive(Side::Left, contents());
    harness.press("Down");
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("C");

    let spec = harness.start_operation().unwrap();
    assert_eq!(spec.sources, Sources::Entries(entries(&["b.txt"], None)));
}

#[test]
fn leaving_the_archive_forgets_the_password() {
    let mut harness = copying_inside();
    harness.operation(OperationEvent::NeedPassword {
        archive: archive(),
        retry: false,
    });
    harness.edit("pw");
    harness.press("Enter");
    harness.finish(Outcome::Completed {
        done: 1,
        skipped: 0,
    });
    harness.press("BS");
    harness.load(Side::Left, vec![file("a.zip"), file("note.txt"), dir("d")]);
    harness.enter_archive(Side::Left, contents());
    harness.press("Down");
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("C");

    let spec = harness.start_operation().unwrap();
    assert_eq!(spec.sources, Sources::Entries(entries(&["b.txt"], None)));
}

#[test]
fn edit_inside_an_archive_extracts_to_a_temporary_directory_and_then_starts_the_editor() {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Down");
    harness.take_effects();

    harness.press("E");

    assert_eq!(
        harness.start_operation(),
        Some(OperationSpec {
            kind: OperationKind::ExtractToTemp,
            sources: Sources::Entries(entries(&["b.txt"], None)),
            dest: None,
            output: None,
        })
    );
    assert_eq!(harness.view().popup.unwrap().title, "Extracting");

    harness.operation(OperationEvent::TempDir(PathBuf::from(TEMP)));
    harness.finish(Outcome::Completed {
        done: 1,
        skipped: 0,
    });

    let effects = harness.take_effects();
    assert!(matches!(
        effects.as_slice(),
        [Effect::ReadDir { .. }, Effect::Spawn { program, args, cwd }]
            if program == "xdg-open"
                && *args == vec![path(TEMP, "b.txt").into_os_string()]
                && cwd.as_os_str() == LEFT
    ));
    assert!(harness.popup().is_none());
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn edit_inside_an_archive_keeps_the_marks_of_the_extracted_entries() {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("E");
    harness.operation(OperationEvent::TempDir(PathBuf::from(TEMP)));
    harness.operation(OperationEvent::ItemDone { index: 0 });
    harness.finish(Outcome::Completed {
        done: 1,
        skipped: 0,
    });

    assert_eq!(harness.marks(Side::Left), ["b.txt"]);
}

#[test]
fn open_with_os_inside_an_archive_extracts_the_cursor_entry() {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("Ctrl+Enter");

    let spec = harness.start_operation().unwrap();
    assert_eq!(spec.kind, OperationKind::ExtractToTemp);
    assert_eq!(
        spec.sources,
        Sources::Entries(entries(&["inner.zip"], None))
    );

    harness.operation(OperationEvent::TempDir(PathBuf::from(TEMP)));
    harness.finish(Outcome::Completed {
        done: 1,
        skipped: 0,
    });

    let effects = harness.take_effects();
    assert!(matches!(
        effects.as_slice(),
        [Effect::ReadDir { .. }, Effect::OpenWithOs(path)] if *path == self::path(TEMP, "inner.zip")
    ));
}

#[test]
fn open_with_os_on_the_parent_row_of_an_archive_root_opens_the_archive_file() {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Space");
    harness.press("Home");
    harness.take_effects();

    harness.press("Ctrl+Enter");

    assert_eq!(harness.take_effects(), vec![Effect::OpenWithOs(archive())]);
    assert_eq!(harness.cursor_name(Side::Left), "..");
}

#[test]
fn open_with_os_on_the_parent_row_inside_an_archive_extracts_the_directory_itself() {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Enter");
    harness.load(Side::Left, vec![file("x")]);
    harness.take_effects();
    assert_eq!(harness.cursor_name(Side::Left), "..");

    harness.press("Ctrl+Enter");

    let spec = harness.start_operation().unwrap();
    assert_eq!(spec.kind, OperationKind::ExtractToTemp);
    assert_eq!(spec.sources, Sources::Entries(entries(&["docs"], None)));

    harness.operation(OperationEvent::TempDir(PathBuf::from(TEMP)));
    harness.finish(Outcome::Completed {
        done: 1,
        skipped: 0,
    });

    let effects = harness.take_effects();
    assert!(matches!(
        effects.as_slice(),
        [Effect::ReadDir { .. }, Effect::OpenWithOs(path)] if *path == self::path(TEMP, "docs")
    ));
    assert_eq!(harness.cursor_name(Side::Left), "..");
}

#[test]
fn an_external_command_inside_an_archive_extracts_the_cursor_and_the_marks() {
    let mut harness = inside();
    harness.config(
        "[commands.show]\nrun = \"show {cursor_path} {marked_paths} {paths} {dir} {other_dir}\"\n[keys.filelist]\n\"F2\" = \"$show\"\n",
    );
    harness.press("Down");
    harness.press("Down");
    harness.press("Space");
    harness.take_effects();

    harness.press("F2");

    let spec = harness.start_operation().unwrap();
    assert_eq!(
        spec.sources,
        Sources::Entries(entries(&["b.txt", "inner.zip"], None))
    );

    harness.operation(OperationEvent::TempDir(PathBuf::from(TEMP)));
    harness.finish(Outcome::Completed {
        done: 2,
        skipped: 0,
    });

    let effects = harness.take_effects();
    let Some(Effect::Spawn { program, args, cwd }) = effects.last() else {
        panic!("expected Spawn, got {effects:?}");
    };
    assert_eq!(program, "show");
    assert_eq!(
        *args,
        vec![
            path(TEMP, "inner.zip").into_os_string(),
            path(TEMP, "b.txt").into_os_string(),
            path(TEMP, "b.txt").into_os_string(),
            PathBuf::from(LEFT).into_os_string(),
            PathBuf::from(RIGHT).into_os_string(),
        ]
    );
    assert_eq!(*cwd, PathBuf::from(LEFT));
}

#[test]
fn an_external_command_on_the_parent_row_inside_an_archive_starts_without_extracting() {
    let mut harness = inside();
    harness.config(
        "[commands.show]\nrun = \"show {cursor_path} {marked_paths} {paths} {dir} {other_dir}\"\n[keys.filelist]\n\"F2\" = \"$show\"\n",
    );
    harness.take_effects();
    assert_eq!(harness.cursor_name(Side::Left), "..");

    harness.press("F2");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::Spawn {
            program: "show".into(),
            args: vec![
                PathBuf::from(LEFT).into_os_string(),
                PathBuf::from(RIGHT).into_os_string(),
            ],
            cwd: PathBuf::from(LEFT),
        }]
    );
    assert!(harness.popup().is_none());
}

#[test]
fn a_cancelled_temporary_extraction_starts_nothing() {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Down");
    harness.press("E");
    harness.operation(OperationEvent::TempDir(PathBuf::from(TEMP)));
    harness.take_effects();

    harness.finish(Outcome::Cancelled { done: 0 });

    let effects = harness.take_effects();
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::Spawn { .. }))
    );
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some("extract cancelled after 0 items")
    );
}

#[test]
fn history_back_inside_an_archive_leaves_it_without_recording_it() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![dir("d")]);
    harness.press("Down");
    harness.press("Enter");
    harness.load(Side::Left, vec![file("a.zip")]);
    harness.press("Down");
    harness.enter_archive(Side::Left, contents());
    assert_eq!(harness.history(Side::Left), [LEFT, &shown(LEFT, "d")]);
    harness.take_effects();

    harness.press("Ctrl+Shift+Left");

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Watch { .. }, Effect::ReadDir { location, .. }]
            if *location == Location::new(PathBuf::from(LEFT))
    ));
    harness.load(Side::Left, vec![dir("d")]);
    assert_eq!(harness.view().left.path, LEFT);
    assert_eq!(harness.history(Side::Left), [LEFT, &shown(LEFT, "d")]);
}

#[test]
fn the_state_saves_the_directory_holding_the_archive() {
    let mut harness = inside();

    harness.press("Alt+Z");
    let effects = harness.take_effects();
    let [Effect::SpawnNewWindow(state)] = effects.as_slice() else {
        panic!("expected SpawnNewWindow, got {effects:?}");
    };
    assert_eq!(state.left.path, Some(PathBuf::from(LEFT)));

    harness.press("Q");

    let effects = harness.take_effects();
    let [Effect::SaveState(state), Effect::Quit] = effects.as_slice() else {
        panic!("expected SaveState and Quit, got {effects:?}");
    };
    assert_eq!(state.left.path, Some(PathBuf::from(LEFT)));
    assert_eq!(state.left.history, vec![PathBuf::from(LEFT)]);
}

#[test]
fn a_change_to_the_archive_reloads_it_and_an_unreadable_one_logs_and_stays() {
    let mut harness = inside();

    harness.send(Msg::Io(IoEvent::LocationChanged {
        side: Side::Left,
        path: archive(),
    }));

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::ReadDir { location, .. }] if *location == root()
    ));

    harness.fail_load(Side::Left, "broken");
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Resolve { path, .. }] if *path == archive()
    ));

    harness.resolve(Side::Left, LEFT, Some("a.zip"));

    assert_eq!(
        harness.log().last().map(String::as_str),
        Some(format!("cannot read {}: broken", shown(LEFT, "a.zip")).as_str())
    );
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Watch {
            side: Side::Left,
            path: archive(),
        }]
    );
    assert_eq!(harness.view().left.path, root().to_string());
}

#[test]
fn a_vanished_archive_falls_back_to_its_directory() {
    let mut harness = inside();
    harness.send(Msg::Io(IoEvent::LocationChanged {
        side: Side::Left,
        path: archive(),
    }));
    harness.fail_load(Side::Left, "gone");
    harness.take_effects();

    harness.resolve(Side::Left, LEFT, None);

    assert_eq!(
        harness.log().last().map(String::as_str),
        Some(format!("{} is not available; moved to {LEFT}", shown(LEFT, "a.zip")).as_str())
    );
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Watch { .. }, Effect::ReadDir { location, .. }]
            if *location == Location::new(PathBuf::from(LEFT))
    ));
}

#[test]
fn go_to_path_naming_the_archive_leaves_it_with_the_cursor_on_the_file() {
    let mut harness = inside();

    harness.press("Shift+J");
    harness.edit(&archive().to_string_lossy());
    harness.press("Enter");
    harness.take_effects();
    harness.resolve(Side::Left, LEFT, Some("a.zip"));

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::Watch { .. }, Effect::ReadDir { location, .. }]
            if *location == Location::new(PathBuf::from(LEFT))
    ));

    harness.load(Side::Left, vec![file("a.zip")]);

    assert_eq!(harness.view().left.path, LEFT);
    assert_eq!(harness.cursor_name(Side::Left), "a.zip");
}

#[test]
fn copy_path_inside_an_archive_spells_the_path_through_the_archive() {
    let mut harness = inside();
    harness.config("[keys.filelist]\n\"Ctrl+C\" = \"copy_path\"\n");
    harness.press("Down");
    harness.press("Down");
    harness.take_effects();

    harness.press("Ctrl+C");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::Clipboard(format!("{}/b.txt", root()))]
    );
}

#[test]
fn stale_archive_replies_are_ignored() {
    let mut harness = outside();
    harness.press("Enter");
    let stale = harness.latest_request(Side::Left);
    harness.press("Esc");
    harness.take_effects();

    harness.send(Msg::Io(IoEvent::DirLoaded {
        side: Side::Left,
        request: stale,
        result: Ok(Directory {
            own: None,
            entries: contents(),
        }),
    }));

    assert_eq!(harness.view().left.path, LEFT);
    assert_eq!(stale, RequestId(stale.0));
}

// The left pane shows a directory with an archive, with the cursor on it.
fn outside() -> Harness {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a.zip"), file("note.txt"), dir("d")]);
    harness.load(Side::Right, vec![file("r.txt")]);
    harness.press("Down");
    harness.press("Down");
    assert_eq!(harness.cursor_name(Side::Left), "a.zip");
    harness.take_effects();
    harness
}

// The left pane is at the root of the archive.
fn inside() -> Harness {
    let mut harness = outside();
    harness.enter_archive(Side::Left, contents());
    harness.take_effects();
    harness
}

// A copy of `b.txt` out of the archive has started.
fn copying_inside() -> Harness {
    let mut harness = inside();
    harness.press("Down");
    harness.press("Down");
    harness.press("Space");
    harness.press("C");
    assert!(harness.start_operation().is_some());
    harness
}

fn contents() -> Vec<crate::entry::Entry> {
    vec![
        dir("docs"),
        file("b.txt"),
        file("inner.zip"),
        file("secret.txt"),
    ]
}

fn archive() -> PathBuf {
    path(LEFT, "a.zip")
}

fn root() -> Location {
    Location::archive_root(archive(), ArchiveFormat::Zip)
}

fn entries(names: &[&str], password: Option<&str>) -> ArchiveEntries {
    ArchiveEntries {
        archive: archive(),
        path: ArchivePath {
            format: ArchiveFormat::Zip,
            inner: Vec::new(),
        },
        names: names.iter().map(|name| (*name).to_owned()).collect(),
        password: password.map(ToOwned::to_owned),
    }
}

fn path(dir: &str, name: &str) -> PathBuf {
    PathBuf::from(dir).join(name)
}

fn shown(dir: &str, name: &str) -> String {
    path(dir, name).display().to_string()
}
