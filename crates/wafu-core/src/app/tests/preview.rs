use std::path::PathBuf;

use crate::app::{Effect, IoEvent, MouseInput, Msg, PreviewError, PreviewSource, Side};
use crate::entry::SpecialKind;
use crate::error::IoError;
use crate::location::{ArchiveFormat, ArchivePath};
use crate::preview::{Document, truncated_note};
use crate::test_support::{Harness, LEFT, RIGHT, file, special};
use crate::view::{PreviewContent, PreviewPlacement};

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
                password: None,
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
        }
    );
    assert_eq!(preview.detail, "");
    // The progress popup is held back by the UI for a moment.
    assert!(view.popup.is_some_and(|popup| popup.progress));
}

#[test]
fn a_loaded_document_shows_its_lines_and_encoding() {
    let mut harness = document();

    let view = harness.view();
    let preview = view.preview.unwrap();
    assert_eq!(preview.detail, "utf-8");
    assert_eq!(harness.preview_lines(), ["l0", "l1", "l2"]);
    assert_eq!(harness.preview_numbers(), ["1", "2", "3"]);
    assert!(matches!(
        preview.content,
        PreviewContent::Lines { digits: 2, .. }
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
fn the_page_follows_the_rows_the_ui_reports() {
    let mut harness = document();

    harness.preview_rows(5);

    assert_eq!(harness.preview_lines(), ["l0", "l1", "l2", "l3", "l4"]);

    harness.press("PageDown");

    assert_eq!(harness.preview_lines(), ["l5", "l6", "l7", "l8", "l9"]);
}

#[test]
fn tab_switches_to_the_hex_dump_and_back_from_the_top() {
    let mut harness = document();
    harness.press("Down");

    harness.press("Tab");

    let lines = harness.preview_lines();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("00000000  6c 30 0a 6c 31 0a"));
    assert!(lines[1].starts_with("00000010 "));
    assert_eq!(harness.preview_numbers(), ["", ""]);
    assert_eq!(harness.view().preview.unwrap().detail, "");

    harness.press("Tab");

    assert_eq!(harness.preview_lines(), ["l0", "l1", "l2"]);
}

#[test]
fn a_binary_file_opens_as_a_hex_dump_with_the_note() {
    let mut harness = opening();
    harness.preview_rows(5);
    let mut raw = b"ab".to_vec();
    raw.push(0);

    harness.preview_loaded(Document::decode(raw, true));

    let lines = harness.preview_lines();
    assert_eq!(lines.len(), 2);
    assert!(lines[0].starts_with("00000000  61 62 00"));
    assert_eq!(lines[1], truncated_note());
    assert_eq!(harness.view().preview.unwrap().detail, "");
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

    harness.fail_preview(PreviewError::Failed(IoError::new("permission denied")));

    assert!(harness.view().preview.is_none());
    assert_eq!(
        harness.log().last().map(String::as_str),
        Some(&*format!(
            "cannot preview {}: permission denied",
            PathBuf::from(LEFT).join("note.txt").display()
        ))
    );
}

#[test]
fn a_stale_document_is_discarded() {
    let mut harness = opening();
    let stale = harness.latest_preview_request();
    harness.press("Esc");
    harness.take_effects();

    harness.send(Msg::Io(IoEvent::PreviewLoaded {
        request: stale,
        result: Ok(Document::decode(b"x".to_vec(), false)),
    }));

    assert!(harness.view().preview.is_none());
}

#[test]
fn hiding_the_log_follows_the_setting_and_the_placement() {
    let mut harness = listing();
    harness.config("[general]\nhide_log_in_preview = true\n");
    harness.press("Down");

    assert!(!harness.view().hide_log);

    harness.press("Enter");
    assert!(harness.view().hide_log);
    harness.press("Esc");

    harness.press("Down");
    harness.press("Enter");

    let view = harness.view();
    assert_eq!(view.preview.unwrap().placement, PreviewPlacement::RightPane);
    assert!(!view.hide_log);

    harness.config("[general]\nhide_log_in_preview = false\n");
    harness.press("Esc");
    harness.press("Up");
    harness.press("Enter");

    assert!(!harness.view().hide_log);
}

#[test]
fn an_image_filling_the_window_hides_the_log() {
    let mut harness = listing();
    harness.config("[general]\nhide_log_in_preview = true\n");
    harness.press("Tab");
    harness.press("Down");

    harness.press("Enter");

    let view = harness.view();
    assert_eq!(view.preview.unwrap().placement, PreviewPlacement::Full);
    assert!(view.hide_log);
}

#[test]
fn the_status_line_stays_in_a_preview_that_hides_the_log() {
    let mut harness = listing();
    harness.config("[general]\nhide_log_in_preview = true\n");
    harness.press("Down");

    harness.press("Enter");
    harness.preview_loaded(Document::decode(b"l0\n".to_vec(), false));

    let view = harness.view();
    assert!(view.hide_log);
    assert_eq!(view.status, "note.txt | -????????? | 0 bytes");
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
                password: None,
            },
            request: harness.latest_preview_request(),
        }]
    );
    let first = harness.latest_preview_request();
    let preview = harness.view().preview.unwrap();
    assert_eq!(preview.placement, PreviewPlacement::RightPane);
    assert_eq!(preview.content, PreviewContent::Image(None));
    assert_eq!(preview.detail, "");

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
    assert_eq!(preview.detail, "");

    harness.press("Up");

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadImage { .. }]
    ));
}

#[test]
fn the_status_line_follows_the_cursor_of_an_image_preview() {
    let mut harness = image();
    assert_eq!(harness.view().status, "one.png | -????????? | 0 bytes");

    harness.press("Down");

    assert!(harness.view().preview.is_some());
    assert_eq!(harness.view().status, "two.JPG | -????????? | 0 bytes");
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
    assert_eq!(preview.detail, "");
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
    assert_eq!(harness.view().preview.unwrap().detail, "png");

    harness.image_loaded(3, 3);

    let PreviewContent::Image(Some(image)) = harness.view().preview.unwrap().content else {
        panic!("expected an image");
    };
    assert_eq!(image.0.width, 3);
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
fn image_keys_are_the_cursor_and_mark_keys_of_the_image_context() {
    let mut harness = image();
    harness.image_loaded(1, 1);

    harness.press("Space");

    assert_eq!(harness.marks(Side::Left), ["one.png"]);
    assert_eq!(harness.cursor_name(Side::Left), "two.JPG");
    harness.press("Up");
    assert_eq!(harness.view().preview.unwrap().detail, "png, marked");
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

    harness.fail_image(PreviewError::Failed(IoError::new("bad png")));

    let message = format!(
        "cannot preview {}: bad png",
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
            format: "png",
        }),
    }));

    assert_eq!(
        harness.view().preview.unwrap().content,
        PreviewContent::Image(None)
    );
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
fn double_click_on_the_left_pane_beside_a_right_pane_image_only_moves_the_cursor() {
    let mut harness = image();

    harness.double_click(Side::Left, 1);

    assert_eq!(harness.cursor_name(Side::Left), "note.txt");
    assert_eq!(
        harness.view().preview.unwrap().content,
        PreviewContent::Image(None)
    );
    assert!(harness.view().left.active);
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

    harness.fail_preview(PreviewError::PasswordRequired { retry: false });

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

    harness.preview_loaded(Document::decode(b"secret".to_vec(), false));
    assert_eq!(harness.preview_lines(), ["secret"]);
    harness.press("Esc");
    harness.take_effects();

    // Opened again, the password is known.
    harness.press("Enter");

    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadPreview { source, .. }] if source.password.as_deref() == Some("pw")
    ));
}

#[test]
fn a_wrong_preview_password_is_asked_again_and_esc_closes() {
    let mut harness = inside_archive();
    harness.press("Enter");
    harness.fail_preview(PreviewError::PasswordRequired { retry: false });
    harness.edit("bad");
    harness.press("Enter");
    harness.take_effects();

    harness.fail_preview(PreviewError::PasswordRequired { retry: true });

    let popup = harness.view().popup.unwrap();
    assert_eq!(popup.message.as_deref(), Some("Wrong password for a.zip"));

    harness.press("Esc");

    assert!(harness.view().popup.is_none());
    assert!(harness.view().preview.is_none());
    assert_eq!(harness.take_effects(), vec![]);
    harness.press("Enter");
    assert!(matches!(
        harness.take_effects().as_slice(),
        [Effect::LoadPreview { source, .. }] if source.password.is_none()
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

    harness.fail_image(PreviewError::PasswordRequired { retry: false });
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
    harness.fail_image(PreviewError::PasswordRequired { retry: false });
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
    assert_eq!(harness.view().preview.unwrap().detail, "png");

    harness.press("Down");
    harness.fail_image(PreviewError::PasswordRequired { retry: false });
    harness.press("Esc");

    let preview = harness.view().preview.unwrap();
    assert!(harness.view().popup.is_none());
    assert_eq!(preview.content, PreviewContent::Image(None));
    assert_eq!(preview.detail, "");
}

#[test]
fn an_encrypted_image_does_not_replace_an_open_popup() {
    let mut harness = inside_archive();
    harness.press("Down");
    harness.press("Enter");
    harness.take_effects();

    harness.press("@");
    harness.fail_image(PreviewError::PasswordRequired { retry: false });

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
    harness.preview_loaded(Document::decode(text.into_bytes(), false));
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

fn entry_source(name: &str, password: Option<&str>) -> PreviewSource {
    PreviewSource {
        path: PathBuf::from(LEFT).join("a.zip"),
        entry: Some((
            ArchivePath {
                format: ArchiveFormat::Zip,
                inner: Vec::new(),
            },
            name.to_owned(),
        )),
        password: password.map(ToOwned::to_owned),
    }
}
