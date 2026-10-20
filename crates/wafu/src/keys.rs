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

// On macOS the Option key composes a character instead of modifying the
// letter, so the composed character is mapped back to its letter. This is the
// US layout; e, i, n, and u are dead keys there and never arrive.
const OPTION_LAYER: [(char, char); 22] = [
    ('å', 'a'),
    ('∫', 'b'),
    ('ç', 'c'),
    ('∂', 'd'),
    ('ƒ', 'f'),
    ('©', 'g'),
    ('˙', 'h'),
    ('∆', 'j'),
    ('˚', 'k'),
    ('¬', 'l'),
    ('µ', 'm'),
    ('ø', 'o'),
    ('π', 'p'),
    ('œ', 'q'),
    ('®', 'r'),
    ('ß', 's'),
    ('†', 't'),
    ('√', 'v'),
    ('∑', 'w'),
    ('≈', 'x'),
    ('¥', 'y'),
    ('Ω', 'z'),
];

/// The modifier flags as Slint reports them, one for one.
#[derive(Debug, Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
pub struct Modifiers {
    pub control: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
}

/// Converts a Slint key event into the input the core expects, or `None` for
/// a key that is not for the core: a Cmd shortcut on macOS, which the OS owns.
///
/// Slint reports Cmd as `control` and the physical Control key as `meta` on
/// macOS, so the two are swapped there. Named keys arrive from Slint as
/// private-use characters; they are replaced with their configuration spelling.
pub fn key_input(text: &str, modifiers: Modifiers) -> Option<KeyInput> {
    let (ctrl, cmd) = if cfg!(target_os = "macos") {
        (modifiers.meta, modifiers.control)
    } else {
        (modifiers.control, false)
    };
    if cmd {
        return None;
    }
    let (text, shift) = match named(text) {
        Some((key, name)) => (name.to_owned(), modifiers.shift || key == Key::Backtab),
        None => (option_layer(text, modifiers.alt), modifiers.shift),
    };
    Some(KeyInput {
        text,
        ctrl,
        shift,
        alt: modifiers.alt,
    })
}

fn named(text: &str) -> Option<(Key, &'static str)> {
    let c = single_char(text)?;
    NAMED.into_iter().find(|(key, _)| char::from(*key) == c)
}

fn option_layer(text: &str, alt: bool) -> String {
    if cfg!(target_os = "macos")
        && alt
        && let Some(c) = single_char(text)
        && let Some((_, letter)) = OPTION_LAYER.iter().find(|(composed, _)| *composed == c)
    {
        return letter.to_string();
    }
    text.to_owned()
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Some(c),
        _ => None,
    }
}
