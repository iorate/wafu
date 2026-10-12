use slint::platform::Key;
use wafu_core::KeyInput;

const NAMED: [(Key, &str); 27] = [
    (Key::Return, "Enter"),
    (Key::Tab, "Tab"),
    (Key::Backtab, "Tab"),
    (Key::Backspace, "BS"),
    (Key::Escape, "Esc"),
    (Key::UpArrow, "Up"),
    (Key::DownArrow, "Down"),
    (Key::LeftArrow, "Left"),
    (Key::RightArrow, "Right"),
    (Key::PageUp, "PageUp"),
    (Key::PageDown, "PageDown"),
    (Key::Home, "Home"),
    (Key::End, "End"),
    (Key::Insert, "Insert"),
    (Key::Delete, "Delete"),
    (Key::F1, "F1"),
    (Key::F2, "F2"),
    (Key::F3, "F3"),
    (Key::F4, "F4"),
    (Key::F5, "F5"),
    (Key::F6, "F6"),
    (Key::F7, "F7"),
    (Key::F8, "F8"),
    (Key::F9, "F9"),
    (Key::F10, "F10"),
    (Key::F11, "F11"),
    (Key::F12, "F12"),
];

/// The physical Control key. Slint reports Cmd as `control` and the physical
/// Control key as `meta` on macOS, so the two are swapped there.
pub fn physical_ctrl(control: bool, meta: bool) -> bool {
    if cfg!(target_os = "macos") {
        meta
    } else {
        control
    }
}

/// Converts a Slint key event into the input the core expects. Named keys
/// arrive from Slint as private-use characters; they are replaced with their
/// configuration spelling.
pub fn key_input(text: &str, ctrl: bool, shift: bool, alt: bool) -> KeyInput {
    let (text, shift) = match named(text) {
        Some((key, name)) => (name.to_owned(), shift || key == Key::Backtab),
        None => (text.to_owned(), shift),
    };
    KeyInput {
        text,
        ctrl,
        shift,
        alt,
    }
}

fn named(text: &str) -> Option<(Key, &'static str)> {
    let mut chars = text.chars();
    let (Some(c), None) = (chars.next(), chars.next()) else {
        return None;
    };
    NAMED.into_iter().find(|(key, _)| char::from(*key) == c)
}
