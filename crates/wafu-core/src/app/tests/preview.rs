use std::path::PathBuf;

use crate::app::{Effect, IoEvent, MouseInput, Msg, PreviewSource, ReadError, Side, SpawnName};
use crate::entry::{Entry, EntryKind, SpecialKind, TargetKind};
use crate::error::IoError;
use crate::location::{ArchiveFormat, ArchivePath, Passwords};
use crate::operation::{
    ArchiveEntries, OperationEvent, OperationKind, OperationSpec, Outcome, Sources,
};
use crate::pane::InFlight;
use crate::preview::{Document, truncated_note};
use crate::test_support::{
    Harness, LEFT, RIGHT, dir, executable, file, passwords, path, shown, special,
};
use crate::view::{PreviewContent, PreviewPlacement};

const TEMP: &str = "/tmp/wafu/x1";

#[test]
fn enter_on_a_text_file_opens_an_empty_preview_and_reads_the_file() {
    let mut harness = listing();
    harness.press("Down");
    harness.take_effects();

    harness.press("Enter");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::LoadPreview {
            source: PreviewSource {
                path: PathBuf::from(LEFT).join("note.txt"),
                entry: None,
                passwords: Passwords::default(),
                encoding: None,
            },
            request: harness.latest_preview_request(),
        }]
    );
    let view = harness.view();
    let preview = view.preview.unwrap();
    assert_eq!(preview.placement, PreviewPlacement::Full);
    assert_eq!(
        preview.content,
        PreviewContent::Lines {
            lines: Vec::new(),
            digits: 0,
            hex_dump: false,
        }
    );
    // The progress popup is held back by the UI for a moment.
    assert!(view.popup.is_some_and(|popup| popup.progress));
}

#[test]
fn enter_on_an_executable_file_opens_the_run_dialog() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![executable("tool")]);
    harness.press("Down");

    harness.press("Enter");

    assert_eq!(harness.view().popup.unwrap().title, "Run");
    assert!(harness.view().preview.is_none());
}

#[test]
fn enter_on_an_executable_image_previews_it() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![executable("one.png")]);
    harness.press("Down");

    harness.press("Enter");

    assert!(harness.popup().is_none());
    assert!(harness.view().preview.is_some());
}

#[test]
fn a_loaded_document_shows_its_lines() {
    let mut harness = document();

    let view = harness.view();
    let preview = view.preview.unwrap();
    assert_eq!(harness.preview_lines(), ["l0", "l1", "l2"]);
    assert_eq!(harness.preview_numbers(), ["1", "2", "3"]);
    assert!(matches!(
        preview.content,
        PreviewContent::Lines {
            digits: 2,
            hex_dump: false,
            ..
        }
    ));

    harness.press("Esc");

    assert!(harness.view().preview.is_none());
    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.cursor_name(Side::Left), "note.txt");
}

#[test]
fn the_keys_scroll_by_lines_and_pages() {
    let mut harness = document();

    harness.press("Down");
    assert_eq!(harness.preview_lines(), ["l1", "l2", "l3"]);

    harness.press("PageDown");
    assert_eq!(harness.preview_lines(), ["l4", "l5", "l6"]);

    harness.press("PageDown");
    harness.press("PageDown");
    harness.press("PageDown");
    assert_eq!(harness.preview_lines(), ["l9"]);

    harness.press("Down");
    assert_eq!(harness.preview_lines(), ["l9"]);

    harness.press("PageUp");
    assert_eq!(harness.preview_lines(), ["l6", "l7", "l8"]);

    harness.press("Home");
    assert_eq!(harness.preview_lines(), ["l0", "l1", "l2"]);

    harness.press("End");
    assert_eq!(harness.preview_lines(), ["l7", "l8", "l9"]);

    harness.press("Up");
    assert_eq!(harness.preview_lines(), ["l6", "l7", "l8"]);

    harness.press("Shift+Up");
    assert_eq!(harness.preview_lines(), ["l6", "l7", "l8"]);

    harness.preview_wheel(-3);
    assert_eq!(harness.preview_lines(), ["l3", "l4", "l5"]);
}

#[test]
fn tab_switches_to_the_hex_dump_and_back_from_the_top() {
    let mut harness = document();
    harness.press("Down");

    harness.press("Tab");

    let lines = harness.preview_lines();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("6c 30 0a 6c 31 0a"));
    assert_eq!(harness.preview_numbers(), ["00000000", "00000010"]);
    let preview = harness.view().preview.unwrap();
    assert!(matches!(
        preview.content,
        PreviewContent::Lines { hex_dump: true, .. }
    ));

    harness.press("Tab");

    assert_eq!(harness.preview_lines(), ["l0", "l1", "l2"]);
}

#[test]
fn c_lists_the_encodings_on_the_one_in_use() {
    let mut harness = document();

    harness.press("C");

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Encoding");
    assert_eq!(popup.cursor, Some(1));
    assert_eq!(popup.fields.len(), 1);
    assert_eq!(
        harness.popup_labels()[..4],
        ["Auto-detect", "UTF-8", "UTF-16LE", "UTF-16BE"]
    );
}

#[test]
fn c_does_nothing_until_the_document_is_read() {
    let mut harness = opening();

    harness.press("C");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn a_picked_encoding_reads_the_file_again_with_it() {
    let mut harness = document();
    harness.press("C");

    harness.edit("shift");
    harness.press("Enter");

    assert_eq!(harness.popup(), None);
    assert_eq!(
        harness.take_effects(),
        vec![Effect::LoadPreview {
            source: PreviewSource {
                path: PathBuf::from(LEFT).join("note.txt"),
                entry: None,
                passwords: Passwords::default(),
                encoding: Some(encoding_rs::SHIFT_JIS),
            },
            request: harness.latest_preview_request(),
        }]
    );
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn c_does_nothing_while_the_file_is_read_again() {
    let mut harness = document();
    harness.press("C");
    harness.edit("shift");
    harness.press("Enter");
    harness.take_effects();

    harness.press("C");

    assert_eq!(harness.popup(), None);
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn a_picked_encoding_shows_text_from_the_top_even_from_the_hex_dump() {
    let mut harness = document();
    harness.press("Tab");
    harness.press("Down");
    harness.press("C");
    harness.edit("shift");
    harness.press("Enter");

    harness.preview_loaded(Document::decode(
        b"a\0\nb\n".to_vec(),
        false,
        Some(encoding_rs::SHIFT_JIS),
    ));

    assert_eq!(harness.preview_lines(), ["a\0", "b"]);
}

#[test]
fn auto_detect_reads_the_file_again_and_lets_it_show_as_binary() {
    let mut harness = document();
    harness.press("C");
    harness.edit("shift");
    harness.press("Enter");
    harness.preview_loaded(Document::decode(
        b"a\0".to_vec(),
        false,
        Some(encoding_rs::SHIFT_JIS),
    ));
    harness.take_effects();

    harness.press("C");
    harness.edit("auto");
    harness.press("Enter");

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadPreview { source, .. }] if source.encoding.is_none()
    ));
    harness.preview_loaded(Document::decode(b"a\0".to_vec(), false, None));
    assert!(harness.preview_lines()[0].starts_with("61 00"));
}

#[test]
fn a_picked_encoding_reads_an_archive_entry_again_with_the_password() {
    let mut harness = inside_archive();
    harness.press("Enter");
    harness.fail_preview(ReadError::PasswordRequired {
        level: 0,
        retry: false,
    });
    harness.edit("pw");
    harness.press("Enter");
    harness.preview_loaded(Document::decode(b"secret".to_vec(), false, None));
    harness.take_effects();

    harness.press("C");
    harness.edit("utf-16le");
    harness.press("Enter");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::LoadPreview {
            source: PreviewSource {
                encoding: Some(encoding_rs::UTF_16LE),
                ..entry_source("b.txt", Some("pw"))
            },
            request: harness.latest_preview_request(),
        }]
    );
}

#[test]
fn e_closes_the_preview_and_opens_the_shown_file_in_the_editor_whatever_is_marked() {
    let mut harness = listing();
    harness.press("Down");
    harness.press("Space");
    harness.press("Down");
    harness.press("Down");
    harness.press("Enter");
    harness.preview_loaded(Document::decode(b"zzz".to_vec(), false, None));
    harness.take_effects();
    assert_eq!(harness.marks(Side::Left), ["note.txt"]);
    assert_eq!(harness.cursor_name(Side::Left), "zzz.txt");

    harness.press("E");

    assert!(harness.view().preview.is_none());
    assert_eq!(
        harness.take_effects(),
        vec![Effect::Spawn {
            name: SpawnName::Editor,
            program: "xdg-open".to_owned(),
            args: vec![path(LEFT, "zzz.txt").into_os_string()],
            cwd: PathBuf::from(LEFT),
        }]
    );
}

#[test]
fn e_on_an_archive_entry_extracts_it_with_the_preview_password_and_then_starts_the_editor() {
    let mut harness = inside_archive();
    harness.press("Enter");
    harness.fail_preview(ReadError::PasswordRequired {
        level: 0,
        retry: false,
    });
    harness.edit("pw");
    harness.press("Enter");
    harness.preview_loaded(Document::decode(b"secret".to_vec(), false, None));
    harness.take_effects();

    harness.press("E");

    assert!(harness.view().preview.is_none());
    assert_eq!(
        harness.start_operation(),
        Some(OperationSpec {
            kind: OperationKind::ExtractToTemp,
            sources: Sources::Entries(ArchiveEntries {
                archive: PathBuf::from(LEFT).join("a.zip"),
                path: ArchivePath {
                    outer: Vec::new(),
                    format: Some(ArchiveFormat::Zip),
                    inner: Vec::new(),
                },
                names: vec!["b.txt".to_owned()],
                passwords: passwords(&[Some("pw")]),
            }),
            dest: None,
            output: None,
        })
    );

    harness.operation(OperationEvent::TempDir(PathBuf::from(TEMP)));
    harness.finish(Outcome::Completed { skipped: 0 });

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::ReadDir { .. }, Effect::Spawn { program, args, .. }]
            if program == "xdg-open" && *args == vec![path(TEMP, "b.txt").into_os_string()]
    ));
}

#[test]
fn a_binary_file_opens_as_a_hex_dump_with_the_note() {
    let mut harness = opening();
    harness.preview_rows(5);
    let mut raw = b"ab".to_vec();
    raw.push(0);

    harness.preview_loaded(Document::decode(raw, true, None));

    let lines = harness.preview_lines();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("61 62 00"));
    assert_eq!(lines[1], truncated_note());
    assert_eq!(harness.preview_numbers(), ["00000000", ""]);
}

#[test]
fn a_slow_read_shows_progress_and_esc_cancels_it() {
    let mut harness = opening();

    let popup = harness.view().popup.unwrap();
    assert!(popup.progress);
    assert_eq!(popup.message.as_deref(), Some("Reading note.txt"));

    harness.press("Esc");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::CancelRead {
            request: harness.latest_preview_request(),
        }]
    );
    assert!(harness.view().preview.is_none());
    assert!(harness.view().popup.is_none());
}

#[test]
fn a_failed_read_is_logged_and_closes_the_preview() {
    let mut harness = opening();

    harness.fail_preview(ReadError::Failed(IoError::new("permission denied")));

    assert!(harness.view().preview.is_none());
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some(&*format!(
            "Cannot preview {}: permission denied",
            PathBuf::from(LEFT).join("note.txt").display()
        ))
    );
}

#[test]
fn a_stale_document_is_discarded() {
    let mut harness = opening();
    let stale = harness.latest_preview_request();
    harness.press("Esc");
    harness.press("Enter");
    harness.take_effects();
    assert_ne!(harness.latest_preview_request(), stale);

    harness.send(Msg::Io(IoEvent::PreviewLoaded {
        request: stale,
        result: Ok(Document::decode(b"x".to_vec(), false, None)),
    }));

    assert_eq!(harness.preview_lines(), Vec::<String>::new());
}

#[test]
fn a_text_preview_hides_the_log_by_its_own_setting() {
    let mut harness = listing();
    harness.config("[preview.text]\nhide_log = true\n");
    harness.press("Down");

    assert!(!harness.view().hide_log);

    harness.press("Enter");

    assert!(harness.view().hide_log);

    harness.press("Esc");
    harness.press("Tab");
    harness.press("Down");
    harness.press("Enter");

    let view = harness.view();
    assert_eq!(view.preview.unwrap().placement, PreviewPlacement::Full);
    assert!(!view.hide_log);
}

#[test]
fn only_an_image_filling_the_window_hides_the_log_by_its_own_setting() {
    let mut harness = listing();
    harness.config("[preview.image]\nhide_log = true\n");
    harness.press("Down");
    harness.press("Enter");

    assert!(!harness.view().hide_log);

    harness.press("Esc");
    harness.press("Down");
    harness.press("Enter");

    let view = harness.view();
    assert_eq!(view.preview.unwrap().placement, PreviewPlacement::RightPane);
    assert!(!view.hide_log);

    harness.press("Esc");
    harness.press("Tab");
    harness.press("Down");
    harness.press("Enter");

    let view = harness.view();
    assert_eq!(view.preview.unwrap().placement, PreviewPlacement::Full);
    assert!(view.hide_log);
}

#[test]
fn an_image_opened_from_the_left_pane_uses_the_right_area_and_follows_the_cursor() {
    let mut harness = listing();
    harness.press("Down");
    harness.press("Down");
    harness.take_effects();

    harness.press("Enter");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::LoadImage {
            source: PreviewSource {
                path: PathBuf::from(LEFT).join("one.png"),
                entry: None,
                passwords: Passwords::default(),
                encoding: None,
            },
            request: harness.latest_preview_request(),
        }]
    );
    let first = harness.latest_preview_request();
    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.placement, PreviewPlacement::RightPane);
    assert_eq!(preview.content, PreviewContent::Image(None));

    // Moving on before the image arrives drops the request.
    harness.press("Down");

    let effects = harness.take_effects();
    assert_eq!(effects.len(), 2);
    assert_eq!(effects[0], Effect::CancelRead { request: first });
    assert!(matches!(
        &effects[1],
        Effect::LoadImage { source, .. } if source.path == PathBuf::from(LEFT).join("two.JPG")
    ));
    assert_eq!(harness.cursor_name(Side::Left), "two.JPG");

    harness.image_loaded(2, 1);

    let PreviewContent::Image(Some(image)) = harness.view().preview.unwrap().content else {
        panic!("expected an image");
    };
    assert_eq!((image.0.width, image.0.height), (2, 1));

    harness.press("Down");

    assert_eq!(harness.cursor_name(Side::Left), "zzz.txt");
    assert_eq!(harness.take_effects(), vec![]);
    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.content, PreviewContent::Image(None));

    harness.press("Up");

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadImage { .. }]
    ));
}

#[test]
fn a_special_file_named_like_an_image_is_not_requested() {
    let mut harness = image();
    harness.image_loaded(2, 1);
    harness.load(
        Side::Left,
        vec![file("one.png"), special("pipe.png", SpecialKind::Fifo)],
    );
    harness.take_effects();

    harness.press("Down");

    assert_eq!(harness.cursor_name(Side::Left), "pipe.png");
    assert_eq!(harness.take_effects(), vec![]);
    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.content, PreviewContent::Image(None));
}

#[test]
fn a_directory_named_like_an_image_is_not_requested() {
    let mut harness = image();
    harness.image_loaded(2, 1);
    harness.load(Side::Left, vec![dir("pics.png"), file("one.png")]);
    harness.take_effects();

    harness.press("Up");

    assert_eq!(harness.cursor_name(Side::Left), "pics.png");
    assert_eq!(harness.take_effects(), vec![]);
    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.content, PreviewContent::Image(None));
}

#[test]
fn a_broken_link_named_like_an_image_is_not_requested() {
    let mut harness = image();
    harness.image_loaded(2, 1);
    let link = Entry {
        kind: EntryKind::Symlink { target: None },
        ..file("two.png")
    };
    harness.load(Side::Left, vec![file("one.png"), link]);
    harness.take_effects();

    harness.press("Down");

    assert_eq!(harness.cursor_name(Side::Left), "two.png");
    assert_eq!(harness.take_effects(), vec![]);
    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.content, PreviewContent::Image(None));
}

#[test]
fn a_link_inside_an_archive_named_like_an_image_is_not_requested() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a.zip")]);
    harness.press("Down");
    harness.press("Enter");
    let link = Entry {
        kind: EntryKind::Symlink {
            target: Some(TargetKind::File { executable: false }),
        },
        ..file("link.png")
    };
    harness.load(Side::Left, vec![file("b.png"), link]);
    harness.press("Down");
    harness.press("Enter");
    harness.image_loaded(2, 1);
    harness.take_effects();

    harness.press("Down");

    assert_eq!(harness.cursor_name(Side::Left), "link.png");
    assert_eq!(harness.take_effects(), vec![]);
    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.content, PreviewContent::Image(None));
}

#[test]
fn the_last_image_stays_until_the_next_one_arrives() {
    let mut harness = image();
    harness.image_loaded(1, 1);

    harness.press("Down");

    assert!(matches!(
        harness.view().preview.unwrap().content,
        PreviewContent::Image(Some(_))
    ));

    harness.image_loaded(3, 3);

    let PreviewContent::Image(Some(image)) = harness.view().preview.unwrap().content else {
        panic!("expected an image");
    };
    assert_eq!(image.0.width, 3);
}

#[test]
fn the_mark_shown_is_that_of_the_image_on_screen() {
    let mut harness = image();
    harness.image_loaded(1, 1);

    harness.press("Space");

    assert_eq!(harness.cursor_name(Side::Left), "two.JPG");
    assert!(harness.view().preview.unwrap().marked);

    harness.image_loaded(1, 1);

    assert!(!harness.view().preview.unwrap().marked);

    harness.press("Up");

    assert!(!harness.view().preview.unwrap().marked);

    harness.image_loaded(1, 1);

    assert!(harness.view().preview.unwrap().marked);
}

#[test]
fn a_preview_showing_nothing_is_not_marked() {
    let mut harness = image();
    harness.image_loaded(1, 1);
    harness.press("Space");
    harness.press("Down");

    assert_eq!(harness.cursor_name(Side::Left), "zzz.txt");
    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.content, PreviewContent::Image(None));
    assert!(!preview.marked);
}

#[test]
fn the_message_of_a_marked_image_is_marked() {
    let mut harness = image();
    harness.image_loaded(1, 1);
    harness.press("Space");
    harness.image_loaded(1, 1);
    harness.press("Up");

    harness.fail_image(ReadError::Failed(IoError::new("bad png")));

    let preview = harness.view().preview.unwrap();
    assert!(matches!(preview.content, PreviewContent::Message(_)));
    assert!(preview.marked);
}

#[test]
fn a_loading_image_shows_progress_until_it_arrives() {
    let mut harness = image();

    let popup = harness.view().popup.unwrap();
    assert!(popup.progress);
    assert_eq!(popup.message.as_deref(), Some("Reading one.png"));

    harness.image_loaded(1, 1);

    assert!(harness.view().popup.is_none());
}

#[test]
fn the_progress_of_each_image_request_has_its_own_generation() {
    let mut harness = image();
    let first = harness.view().popup.unwrap().generation;

    harness.press("Down");

    assert_ne!(harness.view().popup.unwrap().generation, first);
}

#[test]
fn an_image_opened_from_the_right_pane_fills_the_window() {
    let mut harness = listing();
    harness.press("Tab");
    harness.press("Down");
    harness.take_effects();

    harness.press("Enter");

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadImage { source, .. }] if source.path == PathBuf::from(RIGHT).join("r.gif")
    ));
    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.placement, PreviewPlacement::Full);
}

#[test]
fn an_image_opened_from_the_left_pane_fills_the_window_when_set_to() {
    let mut harness = listing();
    harness.config("[preview.image]\nfrom_left = \"both_panes\"\n");
    harness.press("Down");
    harness.press("Down");

    harness.press("Enter");

    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.placement, PreviewPlacement::Full);
}

#[test]
fn an_image_opened_from_the_right_pane_uses_the_left_area_when_set_to() {
    let mut harness = listing();
    harness.config("[preview.image]\nfrom_right = \"other_pane\"\n");
    harness.press("Tab");
    harness.press("Down");

    harness.press("Enter");

    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.placement, PreviewPlacement::LeftPane);
}

#[test]
fn image_keys_are_the_cursor_and_mark_keys_of_the_image_context() {
    let mut harness = image();
    harness.image_loaded(1, 1);

    harness.press("Space");

    assert_eq!(harness.marks(Side::Left), ["one.png"]);
    assert_eq!(harness.cursor_name(Side::Left), "two.JPG");
    harness.press("Up");
    harness.take_effects();

    harness.press("C");
    assert_eq!(harness.take_effects(), vec![]);

    harness.config("[keys.image]\n\"Space\" = \"nop\"\n");
    harness.press("Space");

    assert_eq!(harness.cursor_name(Side::Left), "one.png");
    assert_eq!(harness.marks(Side::Left), ["one.png"]);
    harness.press("Enter");
    assert!(harness.view().preview.is_none());
}

#[test]
fn a_reload_that_moves_the_cursor_changes_the_image() {
    let mut harness = image();

    harness.load(
        Side::Left,
        vec![file("two.JPG"), file("zzz.txt"), file("note.txt")],
    );

    assert_eq!(harness.cursor_name(Side::Left), "two.JPG");
    let effects = harness.take_effects();
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadImage { source, .. } if source.path == PathBuf::from(LEFT).join("two.JPG")
    )));
}

#[test]
fn a_failed_decode_shows_the_message_in_place_of_the_image() {
    let mut harness = image();

    harness.fail_image(ReadError::Failed(IoError::new("bad png")));

    let message = format!(
        "Cannot preview {}: bad png",
        PathBuf::from(LEFT).join("one.png").display()
    );
    assert_eq!(
        harness.view().preview.unwrap().content,
        PreviewContent::Message(message.clone())
    );
    assert_eq!(harness.log().last(), Some(&message));

    harness.press("Down");

    assert!(matches!(
        harness.view().preview.unwrap().content,
        PreviewContent::Image(None)
    ));
}

#[test]
fn a_failed_decode_shows_line_breaks_in_the_name_as_symbols() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a\r.png")]);
    harness.press("Down");
    harness.press("Enter");

    harness.fail_image(ReadError::Failed(IoError::new("bad png")));

    assert_eq!(
        harness.view().preview.unwrap().content,
        PreviewContent::Message(format!(
            "Cannot preview {}: bad png",
            PathBuf::from(LEFT).join("a\u{21B5}.png").display()
        ))
    );
}

#[test]
fn a_stale_image_is_discarded() {
    let mut harness = image();
    let stale = harness.latest_preview_request();
    harness.press("Down");

    harness.send(Msg::Io(IoEvent::ImageLoaded {
        request: stale,
        result: Ok(crate::preview::ImageData {
            width: 1,
            height: 1,
            rgba: vec![0; 4],
        }),
    }));

    assert_eq!(
        harness.view().preview.unwrap().content,
        PreviewContent::Image(None)
    );
}

#[test]
fn esc_cancels_a_move_started_beside_an_image_and_keeps_the_preview() {
    let mut harness = image();
    harness.changed(Side::Left);
    harness.fail_load(Side::Left, "gone");
    let request = harness.latest_resolve(Side::Left);
    harness.take_effects();

    harness.press("Esc");

    assert!(matches!(
        harness.app().pane(Side::Left).loading(),
        Some(InFlight::Refresh { .. })
    ));
    assert!(harness.view().preview.is_some());
    assert!(
        harness
            .take_effects()
            .contains(&Effect::CancelRead { request })
    );
}

#[test]
fn the_mouse_is_ignored_during_a_move_started_beside_an_image() {
    let mut harness = image();
    harness.changed(Side::Left);
    harness.fail_load(Side::Left, "gone");

    harness.click(Side::Left, 3);
    harness.preview_double_click();

    assert_eq!(harness.cursor_name(Side::Left), "one.png");
    assert!(harness.view().preview.is_some());
}

#[test]
fn double_click_on_a_document_preview_closes_it() {
    let mut harness = document();

    harness.preview_double_click();

    assert!(harness.view().preview.is_none());
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn double_click_on_an_image_preview_closes_it_and_cancels_the_read() {
    let mut harness = image();
    let request = harness.latest_preview_request();

    harness.preview_double_click();

    assert!(harness.view().preview.is_none());
    assert_eq!(harness.take_effects(), vec![Effect::CancelRead { request }]);
}

#[test]
fn the_left_pane_takes_clicks_and_the_wheel_beside_a_right_pane_image() {
    let mut harness = image();
    harness.viewport(Side::Left, 2);

    harness.click(Side::Left, 3);

    assert_eq!(harness.cursor_name(Side::Left), "two.JPG");
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::CancelRead { .. }, Effect::LoadImage { source, .. }]
            if source.path == PathBuf::from(LEFT).join("two.JPG")
    ));

    harness.wheel(Side::Left, 1);

    assert_eq!(harness.view().left.scroll_top, 3);
    assert_eq!(harness.cursor_name(Side::Left), "two.JPG");
    assert!(harness.view().preview.is_some());

    harness.scroll_to(Side::Left, 0);

    assert_eq!(harness.view().left.scroll_top, 0);
    assert_eq!(harness.cursor_name(Side::Left), "two.JPG");
}

#[test]
fn double_click_on_the_left_pane_beside_a_right_pane_image_moves_the_cursor_and_closes_it() {
    let mut harness = image();
    let request = harness.latest_preview_request();

    harness.double_click(Side::Left, 1);

    assert_eq!(harness.cursor_name(Side::Left), "note.txt");
    assert!(harness.view().preview.is_none());
    assert!(harness.view().left.active);
    assert_eq!(harness.take_effects(), vec![Effect::CancelRead { request }]);
}

#[test]
fn the_right_pane_and_right_clicks_are_ignored_beside_a_right_pane_image() {
    let mut harness = image();

    harness.click(Side::Right, 1);
    harness.double_click(Side::Right, 1);
    harness.send(Msg::Mouse(MouseInput::RightClick {
        side: Side::Left,
        row: 1,
    }));

    assert!(harness.view().left.active);
    assert_eq!(harness.cursor_name(Side::Left), "one.png");
    assert_eq!(harness.cursor_name(Side::Right), "..");
    assert_eq!(harness.take_effects(), vec![]);
}

#[test]
fn the_right_pane_takes_the_mouse_beside_a_left_pane_image() {
    let mut harness = listing();
    harness.config("[preview.image]\nfrom_right = \"other_pane\"\n");
    harness.press("Tab");
    harness.press("Down");
    harness.press("Enter");

    harness.click(Side::Left, 1);

    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert!(harness.view().preview.is_some());

    harness.click(Side::Right, 0);

    assert_eq!(harness.cursor_name(Side::Right), "..");

    harness.double_click(Side::Right, 1);

    assert_eq!(harness.cursor_name(Side::Right), "r.gif");
    assert!(harness.view().preview.is_none());
}

#[test]
fn dividers_and_the_log_take_the_mouse_under_a_preview() {
    let mut harness = image();
    harness.config("[filelist]\na = 1\nb = 2\n");

    harness.send(Msg::Mouse(MouseInput::SplitTo { percent: 70 }));
    harness.send(Msg::Mouse(MouseInput::LogRowsTo { rows: 1 }));
    harness.send(Msg::Mouse(MouseInput::LogScrollTo { top: 0 }));
    harness.send(Msg::Mouse(MouseInput::LogWheel { rows: 1 }));

    let view = harness.view();
    assert_eq!(view.split_percent, 70);
    assert_eq!(view.log_rows, 1);
    assert_eq!(view.log.scroll_top, 1);
    assert!(view.preview.is_some());
}

#[test]
fn layout_keys_work_during_an_image_preview_whatever_its_placement() {
    let mut harness = image();

    harness.press("Alt+Right");
    harness.press("Alt+Up");

    assert_eq!(harness.view().split_percent, 55);
    assert_eq!(harness.view().log_rows, 7);
    assert!(harness.view().preview.is_some());

    harness.press("Esc");
    harness.press("Tab");
    harness.press("Down");
    harness.press("Enter");
    harness.config("[preview.image]\nhide_log = true\n");
    assert_eq!(
        harness.view().preview.unwrap().placement,
        PreviewPlacement::Full
    );

    harness.press("Alt+Right");
    harness.press("Alt+Up");

    assert_eq!(harness.view().split_percent, 60);
    assert_eq!(harness.view().log_rows, 8);
}

#[test]
fn mouse_clicks_are_ignored_during_a_full_image_preview() {
    let mut harness = listing();
    harness.press("Tab");
    harness.press("Down");
    harness.press("Enter");

    harness.click(Side::Left, 1);
    harness.wheel(Side::Left, 1);
    harness.scroll_to(Side::Left, 1);
    harness.double_click(Side::Left, 1);

    assert!(harness.view().right.active);
    assert_eq!(harness.cursor_name(Side::Left), "..");
    assert_eq!(harness.view().left.scroll_top, 0);
    assert!(harness.view().preview.is_some());
}

#[test]
fn a_file_inside_an_archive_is_read_as_an_entry_with_the_known_password() {
    let mut harness = inside_archive();

    harness.press("Enter");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::LoadPreview {
            source: entry_source("b.txt", None),
            request: harness.latest_preview_request(),
        }]
    );

    harness.fail_preview(ReadError::PasswordRequired {
        level: 0,
        retry: false,
    });

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.title, "Password");
    assert_eq!(popup.message.as_deref(), Some("a.zip"));
    assert!(harness.view().preview.is_some());

    harness.press("Enter");
    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.view().popup.unwrap().title, "Password");

    harness.edit("pw");
    harness.press("Enter");

    assert!(harness.view().popup.is_some_and(|popup| popup.progress));
    assert_eq!(
        harness.take_effects(),
        vec![Effect::LoadPreview {
            source: entry_source("b.txt", Some("pw")),
            request: harness.latest_preview_request(),
        }]
    );

    harness.preview_loaded(Document::decode(b"secret".to_vec(), false, None));
    assert_eq!(harness.preview_lines(), ["secret"]);
    harness.press("Esc");
    harness.take_effects();

    // Opened again, the password is known.
    harness.press("Enter");

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadPreview { source, .. }] if source.passwords.get(0) == Some("pw")
    ));
}

#[test]
fn closing_a_preview_inside_an_archive_releases_the_archive() {
    let mut harness = inside_archive();
    harness.press("Enter");
    harness.preview_loaded(Document::decode(b"b".to_vec(), false, None));
    harness.take_effects();

    harness.press("Esc");

    assert_eq!(harness.take_effects(), vec![Effect::ReleasePreview]);
}

#[test]
fn an_image_preview_releases_the_archive_only_once_the_pane_leaves_it() {
    let mut harness = inside_archive();
    harness.press("Down");
    harness.press("Enter");
    harness.image_loaded(1, 1);
    harness.press("Up");
    assert!(!harness.take_effects().contains(&Effect::ReleasePreview));

    harness.changed(Side::Left);
    harness.fail_load(Side::Left, "No such file or directory");
    harness.resolve(Side::Left, LEFT, None);
    harness.load(Side::Left, vec![file("note.txt")]);

    assert!(harness.view().preview.is_some());
    assert!(harness.take_effects().contains(&Effect::ReleasePreview));
}

#[test]
fn a_preview_password_for_an_outer_archive_is_asked_for_it_and_sent_at_its_level() {
    let mut harness = inside_nested_archive();
    harness.press("Enter");

    harness.fail_preview(ReadError::PasswordRequired {
        level: 0,
        retry: false,
    });

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.message.as_deref(), Some("a.zip"));
    harness.take_effects();
    harness.edit("pw");
    harness.press("Enter");
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadPreview { source, .. }] if source.passwords == passwords(&[Some("pw")])
    ));
}

#[test]
fn an_image_password_in_a_nested_archive_is_asked_for_that_archive() {
    let mut harness = inside_nested_archive();
    harness.press("Down");
    harness.press("Enter");

    harness.fail_image(ReadError::PasswordRequired {
        level: 1,
        retry: false,
    });

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.message.as_deref(), Some("n.zip"));
}

#[test]
fn a_wrong_preview_password_is_asked_again_and_esc_closes() {
    let mut harness = inside_archive();
    harness.press("Enter");
    harness.fail_preview(ReadError::PasswordRequired {
        level: 0,
        retry: false,
    });
    harness.edit("bad");
    harness.press("Enter");
    harness.take_effects();

    harness.fail_preview(ReadError::PasswordRequired {
        level: 0,
        retry: true,
    });

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.message.as_deref(), Some("Wrong password for a.zip"));

    harness.press("Esc");

    assert!(harness.view().popup.is_none());
    assert!(harness.view().preview.is_none());
    assert_eq!(harness.take_effects(), vec![Effect::ReleasePreview]);
    harness.press("Enter");
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadPreview { source, .. }] if source.passwords == Passwords::default()
    ));
}

#[test]
fn an_encrypted_image_asks_once_and_shows_nothing_on_esc() {
    let mut harness = inside_archive();
    harness.press("Down");
    harness.take_effects();
    harness.press("Enter");
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadImage { source, .. }] if *source == entry_source("c.png", None)
    ));

    harness.fail_image(ReadError::PasswordRequired {
        level: 0,
        retry: false,
    });
    harness.press("Esc");

    assert!(harness.view().popup.is_none());
    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(
        harness.view().preview.unwrap().content,
        PreviewContent::Image(None)
    );

    harness.press("Up");
    harness.press("Down");
    harness.take_effects();
    harness.fail_image(ReadError::PasswordRequired {
        level: 0,
        retry: false,
    });
    harness.edit("pw");
    harness.press("Enter");

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadImage { source, .. }] if *source == entry_source("c.png", Some("pw"))
    ));
}

#[test]
fn esc_on_the_password_of_an_encrypted_image_clears_the_previous_image() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a.zip")]);
    harness.press("Down");
    harness.press("Enter");
    harness.load(Side::Left, vec![file("b.png"), file("c.png")]);
    harness.press("Down");
    harness.press("Enter");
    harness.image_loaded(1, 1);

    harness.press("Down");
    harness.fail_image(ReadError::PasswordRequired {
        level: 0,
        retry: false,
    });
    harness.press("Esc");

    let preview = harness.view().preview.unwrap();
    assert!(harness.view().popup.is_none());
    assert_eq!(preview.content, PreviewContent::Image(None));
}

#[test]
fn an_encrypted_image_does_not_replace_an_open_popup() {
    let mut harness = inside_archive();
    harness.press("Down");
    harness.press("Enter");
    harness.take_effects();

    harness.press("@");
    harness.fail_image(ReadError::PasswordRequired {
        level: 0,
        retry: false,
    });

    assert_eq!(harness.view().popup.unwrap().title, "Mark");
    harness.edit("*.png");
    harness.press("Enter");
    assert_eq!(harness.marks(Side::Left), ["c.png"]);
    assert!(harness.view().popup.is_none());
    assert_eq!(
        harness.view().preview.unwrap().content,
        PreviewContent::Image(None)
    );
}

#[test]
fn a_link_inside_an_archive_is_not_previewed() {
    let mut harness = inside_archive();
    harness.press("Down");
    harness.press("Down");
    harness.take_effects();

    harness.press("Enter");

    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.view().preview.is_none());
}

#[test]
fn v_opens_an_image_in_the_text_preview() {
    let mut harness = listing();
    harness.press("Down");
    harness.press("Down");
    harness.take_effects();

    harness.press("V");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::LoadPreview {
            source: PreviewSource {
                path: PathBuf::from(LEFT).join("one.png"),
                entry: None,
                passwords: Passwords::default(),
                encoding: None,
            },
            request: harness.latest_preview_request(),
        }]
    );
    assert!(matches!(
        harness.view().preview.unwrap().content,
        PreviewContent::Lines { .. }
    ));
}

#[test]
fn v_on_an_archive_inside_an_archive_opens_it_in_the_text_preview() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a.zip")]);
    harness.press("Down");
    harness.press("Enter");
    harness.load(Side::Left, vec![file("inner.zip")]);
    harness.press("Down");
    harness.take_effects();

    harness.press("V");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::LoadPreview {
            source: entry_source("inner.zip", None),
            request: harness.latest_preview_request(),
        }]
    );
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn v_on_the_parent_row_does_nothing() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a.txt")]);
    harness.take_effects();

    harness.press("V");

    assert_eq!(harness.take_effects(), vec![]);
    assert_eq!(harness.view().left.path, LEFT);
    assert!(harness.view().preview.is_none());
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn v_on_the_parent_row_of_an_archive_root_opens_the_archive_file() {
    let mut harness = inside_archive();
    harness.press("Home");
    harness.take_effects();

    harness.press("V");

    assert_eq!(
        harness.take_effects(),
        vec![Effect::LoadPreview {
            source: PreviewSource {
                path: PathBuf::from(LEFT).join("a.zip"),
                entry: None,
                passwords: Passwords::default(),
                encoding: None,
            },
            request: harness.latest_preview_request(),
        }]
    );
}

#[test]
fn v_on_a_directory_does_nothing() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![dir("d")]);
    harness.press("Down");
    harness.take_effects();

    harness.press("V");

    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.view().preview.is_none());
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn v_on_a_broken_link_logs_an_error() {
    let mut harness = Harness::start(LEFT, RIGHT);
    let link = Entry {
        kind: EntryKind::Symlink { target: None },
        ..file("dead.png")
    };
    harness.load(Side::Left, vec![link]);
    harness.press("Down");
    harness.take_effects();

    harness.press("V");

    assert_eq!(
        harness.log(),
        [format!(
            "Cannot open {}: the symbolic link is broken",
            shown(LEFT, "dead.png")
        )]
    );
    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.view().preview.is_none());
}

#[test]
fn v_on_a_special_file_logs_an_error() {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![special("pipe", SpecialKind::Fifo)]);
    harness.press("Down");
    harness.take_effects();

    harness.press("V");

    assert_eq!(
        harness.log(),
        [format!(
            "Cannot open {}: not a regular file",
            shown(LEFT, "pipe")
        )]
    );
    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.view().preview.is_none());
}

#[test]
fn v_on_a_link_inside_an_archive_does_nothing() {
    let mut harness = inside_archive();
    harness.press("Down");
    harness.press("Down");
    harness.take_effects();

    harness.press("V");

    assert_eq!(harness.take_effects(), vec![]);
    assert!(harness.view().preview.is_none());
    assert_eq!(harness.log(), Vec::<String>::new());
}

#[test]
fn text_keys_come_from_the_text_table_in_both_modes() {
    let mut harness = document();
    harness.config(
        "[keys.filelist]\n\"Down\" = \"nop\"\n[keys.text]\n\"J\" = \"preview_scroll_down\"\n",
    );

    harness.press("J");
    harness.press("Down");
    assert_eq!(harness.preview_lines(), ["l2", "l3", "l4"]);

    harness.press("Tab");
    harness.press("J");
    assert_eq!(harness.preview_numbers()[0], "00000010");

    harness.press("Alt+Up");
    assert_eq!(harness.view().log_rows, 7);
}

#[test]
fn image_keys_come_from_the_image_table() {
    let mut harness = image();
    harness.config("[keys.filelist]\n\"Down\" = \"nop\"\n[keys.image]\n\"Esc\" = \"nop\"\n");

    harness.press("Down");
    assert_eq!(harness.cursor_name(Side::Left), "two.JPG");

    harness.press("Esc");
    assert!(harness.view().preview.is_some());
    harness.press("Enter");
    assert!(harness.view().preview.is_none());
}

#[test]
fn the_palette_opens_during_a_preview_with_its_commands() {
    let mut harness = document();

    harness.press("F1");

    let labels: Vec<String> = harness
        .popup_items()
        .iter()
        .map(|item| item.label.clone())
        .collect();
    assert!(labels.contains(&"preview_scroll_down".to_owned()));

    harness.edit("close_pre");
    harness.press("Enter");

    assert_eq!(harness.popup(), None);
    assert!(harness.view().preview.is_none());
}

// The left pane lists a text file, two images, and another text file, in that order.
fn listing() -> Harness {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(
        Side::Left,
        vec![
            file("note.txt"),
            file("one.png"),
            file("two.JPG"),
            file("zzz.txt"),
        ],
    );
    harness.load(Side::Right, vec![file("r.gif")]);
    harness.take_effects();
    harness
}

// The text file is being read.
fn opening() -> Harness {
    let mut harness = listing();
    harness.press("Down");
    harness.press("Enter");
    harness.take_effects();
    harness
}

// The text file is shown, ten lines long, three rows at a time.
fn document() -> Harness {
    let mut harness = opening();
    let text = (0..10).fold(String::new(), |text, i| format!("{text}l{i}\n"));
    harness.preview_loaded(Document::decode(text.into_bytes(), false, None));
    harness.preview_rows(3);
    harness.take_effects();
    harness
}

// The first image was opened from the left pane and is being read.
fn image() -> Harness {
    let mut harness = listing();
    harness.press("Down");
    harness.press("Down");
    harness.press("Enter");
    harness.take_effects();
    harness
}

// The left pane is at the root of `a.zip`, on `b.txt`.
fn inside_archive() -> Harness {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a.zip")]);
    harness.press("Down");
    harness.press("Enter");
    let link = crate::entry::Entry {
        kind: crate::entry::EntryKind::Symlink {
            target: Some(crate::entry::TargetKind::File { executable: false }),
        },
        ..file("link")
    };
    harness.load(Side::Left, vec![file("b.txt"), file("c.png"), link]);
    harness.press("Down");
    harness.take_effects();
    assert_eq!(harness.cursor_name(Side::Left), "b.txt");
    harness
}

// The left pane is inside `n.zip`, an entry of `a.zip`, with the cursor on
// `b.txt` and `c.png` below it.
fn inside_nested_archive() -> Harness {
    let mut harness = Harness::start(LEFT, RIGHT);
    harness.load(Side::Left, vec![file("a.zip")]);
    harness.press("Down");
    harness.press("Enter");
    harness.load(Side::Left, vec![file("n.zip")]);
    harness.press("Down");
    harness.press("Enter");
    harness.load(Side::Left, vec![file("b.txt"), file("c.png")]);
    harness.press("Down");
    harness.take_effects();
    harness
}

fn entry_source(name: &str, password: Option<&str>) -> PreviewSource {
    PreviewSource {
        path: PathBuf::from(LEFT).join("a.zip"),
        entry: Some((
            ArchivePath {
                outer: Vec::new(),
                format: Some(ArchiveFormat::Zip),
                inner: Vec::new(),
            },
            name.to_owned(),
        )),
        passwords: passwords(&[password]),
        encoding: None,
    }
}
