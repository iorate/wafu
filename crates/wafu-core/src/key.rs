use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// A normalized key press, as written in the configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Key {
    pub base: KeyBase,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KeyBase {
    /// An upper-case letter, a digit, or a symbol as typed.
    Char(char),
    Named(NamedKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Enter,
    Space,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    PageUp,
    PageDown,
    Home,
    End,
    Insert,
    Delete,
    F(u8),
}

/// A key press as delivered by the UI, before normalization.
///
/// `text` is the typed text, or the configuration spelling of a named key
/// (`Up`, `Enter`, ...). `ctrl` is the physical Control key on every OS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyInput {
    pub text: String,
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
#[error("invalid key: {0:?}")]
pub struct KeyParseError(String);

impl Key {
    /// Normalizes a key press from the UI. Returns `None` for input that
    /// cannot be a key binding, such as text committed by an input method.
    #[must_use]
    pub fn from_input(input: &KeyInput) -> Option<Self> {
        let mut chars = input.text.chars();
        let (base, shift) = match (chars.next(), chars.next()) {
            (Some(' '), None) => (KeyBase::Named(NamedKey::Space), input.shift),
            // Modifier keys pressed on their own arrive as control characters.
            (Some(c), None) if c.is_control() => return None,
            (Some(c), None) if c.is_ascii_alphabetic() => {
                (KeyBase::Char(c.to_ascii_uppercase()), input.shift)
            }
            (Some(c), None) => (KeyBase::Char(c), false),
            _ => {
                let named = input.text.parse::<NamedKey>().ok()?;
                if named.to_string() != input.text {
                    return None;
                }
                (KeyBase::Named(named), input.shift)
            }
        };
        Some(Self {
            base,
            ctrl: input.ctrl,
            shift,
            alt: input.alt,
        })
    }
}

impl FromStr for Key {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let error = || KeyParseError(s.to_owned());
        let (modifiers, base) = if s == "+" {
            (None, "+")
        } else if let Some(rest) = s.strip_suffix("++") {
            (Some(rest), "+")
        } else {
            match s.rsplit_once('+') {
                Some((modifiers, base)) => (Some(modifiers), base),
                None => (None, s),
            }
        };
        let (mut ctrl, mut shift, mut alt) = (false, false, false);
        for modifier in modifiers.into_iter().flat_map(|m| m.split('+')) {
            let flag = match modifier.to_ascii_lowercase().as_str() {
                "ctrl" => &mut ctrl,
                "shift" => &mut shift,
                "alt" => &mut alt,
                _ => return Err(error()),
            };
            if *flag {
                return Err(error());
            }
            *flag = true;
        }
        let mut chars = base.chars();
        let base = match (chars.next(), chars.next()) {
            (Some(c), None) if c.is_ascii_alphabetic() => KeyBase::Char(c.to_ascii_uppercase()),
            // Symbols are written as the typed character, never with Shift.
            (Some(_), None) if shift => return Err(error()),
            (Some(c), None) => KeyBase::Char(c),
            (Some(_), Some(_)) => KeyBase::Named(base.parse().map_err(|_| error())?),
            (None, _) => return Err(error()),
        };
        Ok(Self {
            base,
            ctrl,
            shift,
            alt,
        })
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        match self.base {
            KeyBase::Char(c) => write!(f, "{c}"),
            KeyBase::Named(named) => write!(f, "{named}"),
        }
    }
}

impl FromStr for NamedKey {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let named = match s.to_ascii_lowercase().as_str() {
            "enter" => Self::Enter,
            "space" => Self::Space,
            "tab" => Self::Tab,
            "bs" => Self::Backspace,
            "esc" => Self::Escape,
            "up" => Self::Up,
            "down" => Self::Down,
            "left" => Self::Left,
            "right" => Self::Right,
            "pageup" => Self::PageUp,
            "pagedown" => Self::PageDown,
            "home" => Self::Home,
            "end" => Self::End,
            "insert" => Self::Insert,
            "delete" => Self::Delete,
            lower => match lower
                .strip_prefix('f')
                .filter(|digits| digits.bytes().all(|b| b.is_ascii_digit()))
                .and_then(|digits| digits.parse::<u8>().ok())
            {
                Some(n @ 1..=12) if !lower.starts_with("f0") => Self::F(n),
                _ => return Err(KeyParseError(s.to_owned())),
            },
        };
        Ok(named)
    }
}

impl fmt::Display for NamedKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Enter => f.write_str("Enter"),
            Self::Space => f.write_str("Space"),
            Self::Tab => f.write_str("Tab"),
            Self::Backspace => f.write_str("BS"),
            Self::Escape => f.write_str("Esc"),
            Self::Up => f.write_str("Up"),
            Self::Down => f.write_str("Down"),
            Self::Left => f.write_str("Left"),
            Self::Right => f.write_str("Right"),
            Self::PageUp => f.write_str("PageUp"),
            Self::PageDown => f.write_str("PageDown"),
            Self::Home => f.write_str("Home"),
            Self::End => f.write_str("End"),
            Self::Insert => f.write_str("Insert"),
            Self::Delete => f.write_str("Delete"),
            Self::F(n) => write!(f, "F{n}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_letter_normalizes_to_upper_case() {
        assert_eq!("a".parse::<Key>(), Ok(key(KeyBase::Char('A'))));
        assert_eq!("A".parse::<Key>(), Ok(key(KeyBase::Char('A'))));
    }

    #[test]
    fn parse_modifiers_in_any_order_and_case() {
        let expected = Key {
            ctrl: true,
            shift: true,
            alt: true,
            ..key(KeyBase::Named(NamedKey::Left))
        };

        assert_eq!("Ctrl+Shift+Alt+Left".parse::<Key>(), Ok(expected));
        assert_eq!("alt+SHIFT+ctrl+left".parse::<Key>(), Ok(expected));
    }

    #[test]
    fn parse_symbols_and_digits() {
        assert_eq!("@".parse::<Key>(), Ok(key(KeyBase::Char('@'))));
        assert_eq!("!".parse::<Key>(), Ok(key(KeyBase::Char('!'))));
        assert_eq!("~".parse::<Key>(), Ok(key(KeyBase::Char('~'))));
        assert_eq!("\\".parse::<Key>(), Ok(key(KeyBase::Char('\\'))));
        assert_eq!(
            "Ctrl+1".parse::<Key>(),
            Ok(Key {
                ctrl: true,
                ..key(KeyBase::Char('1'))
            })
        );
    }

    #[test]
    fn parse_plus_as_a_key() {
        assert_eq!("+".parse::<Key>(), Ok(key(KeyBase::Char('+'))));
        assert_eq!(
            "Ctrl++".parse::<Key>(),
            Ok(Key {
                ctrl: true,
                ..key(KeyBase::Char('+'))
            })
        );
    }

    #[test]
    fn parse_function_keys() {
        assert_eq!("F1".parse::<Key>(), Ok(key(KeyBase::Named(NamedKey::F(1)))));
        assert_eq!(
            "f12".parse::<Key>(),
            Ok(key(KeyBase::Named(NamedKey::F(12))))
        );
        assert!("F0".parse::<Key>().is_err());
        assert!("F13".parse::<Key>().is_err());
        assert!("F01".parse::<Key>().is_err());
        assert!("F+1".parse::<Key>().is_err());
        assert!("F-1".parse::<Key>().is_err());
        assert!("F 1".parse::<Key>().is_err());
    }

    #[test]
    fn parse_rejects_invalid_strings() {
        for text in [
            "",
            "Ctrl+",
            "Ctrl+Ctrl+A",
            "+A",
            "Ctrl++A",
            "++",
            "Ctrl+++",
            "Meta+A",
            "Shift+!",
            "Shift+1",
            "Foo",
            "ab",
        ] {
            assert!(text.parse::<Key>().is_err(), "{text:?} should be rejected");
        }
    }

    #[test]
    fn display_round_trips_through_parse() {
        for text in [
            "A",
            "Shift+A",
            "Ctrl+Space",
            "Alt+Z",
            "Ctrl+Shift+Right",
            "@",
            "!",
            "~",
            "Ctrl+1",
            "+",
            "Ctrl+:",
            "F5",
            "BS",
            "Esc",
            "PageDown",
        ] {
            let key: Key = text.parse().unwrap();
            assert_eq!(key.to_string(), text);
            assert_eq!(key.to_string().parse::<Key>(), Ok(key));
        }
    }

    #[test]
    fn from_input_upper_cases_letters_and_keeps_shift() {
        assert_eq!(
            Key::from_input(&input("a", false, false)),
            Some(key(KeyBase::Char('A')))
        );
        assert_eq!(
            Key::from_input(&input("A", false, true)),
            Some(Key {
                shift: true,
                ..key(KeyBase::Char('A'))
            })
        );
    }

    #[test]
    fn from_input_drops_shift_on_symbols_and_digits() {
        assert_eq!(
            Key::from_input(&input("!", false, true)),
            Some(key(KeyBase::Char('!')))
        );
        assert_eq!(
            Key::from_input(&input("1", false, false)),
            Some(key(KeyBase::Char('1')))
        );
    }

    #[test]
    fn from_input_maps_space_character_to_space_key() {
        assert_eq!(
            Key::from_input(&input(" ", true, false)),
            Some(Key {
                ctrl: true,
                ..key(KeyBase::Named(NamedKey::Space))
            })
        );
    }

    #[test]
    fn from_input_ignores_control_characters() {
        assert_eq!(Key::from_input(&input("\u{11}", true, false)), None);
        assert_eq!(Key::from_input(&input("\u{10}", false, true)), None);
        assert_eq!(Key::from_input(&input("\n", false, false)), None);
    }

    #[test]
    fn from_input_accepts_named_key_spellings() {
        assert_eq!(
            Key::from_input(&input("PageUp", false, true)),
            Some(Key {
                shift: true,
                ..key(KeyBase::Named(NamedKey::PageUp))
            })
        );
    }

    #[test]
    fn from_input_accepts_canonical_named_keys_only() {
        assert_eq!(
            Key::from_input(&input("Up", false, false)),
            Some(key(KeyBase::Named(NamedKey::Up)))
        );
        assert_eq!(
            Key::from_input(&input("F12", false, false)),
            Some(key(KeyBase::Named(NamedKey::F(12))))
        );
        for text in ["up", "END", "tab", "bs", "f1", "F+1"] {
            assert_eq!(
                Key::from_input(&input(text, false, false)),
                None,
                "{text:?}"
            );
        }
    }

    #[test]
    fn from_input_ignores_unknown_text() {
        assert_eq!(Key::from_input(&input("", false, false)), None);
        assert_eq!(Key::from_input(&input("日本語", false, false)), None);
    }

    fn key(base: KeyBase) -> Key {
        Key {
            base,
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    fn input(text: &str, ctrl: bool, shift: bool) -> KeyInput {
        KeyInput {
            text: text.to_owned(),
            ctrl,
            shift,
            alt: false,
        }
    }
}
