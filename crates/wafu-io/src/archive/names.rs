/// Decodes an entry name: UTF-8 when it is valid, else the system's default
/// encoding (the ANSI code page on Windows), with unreadable parts replaced.
pub(crate) fn decode_name(raw: &[u8]) -> String {
    match std::str::from_utf8(raw) {
        Ok(name) => name.to_owned(),
        Err(_) => decode_with_locale(raw),
    }
}

/// Splits a name at `/` and `\` (old Windows tools wrote the latter),
/// dropping empty parts such as the trailing separator of a directory.
pub(crate) fn split_name(name: &str) -> Vec<String> {
    name.split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

/// Whether a component may be used as one path element under the
/// destination. Anything that could reach outside is refused.
pub(crate) fn check_component(component: &str) -> bool {
    !(component.is_empty()
        || component == "."
        || component == ".."
        || component.contains(['\0', '/', '\\'])
        || (cfg!(windows) && component.contains(':')))
}

#[cfg(windows)]
fn decode_with_locale(raw: &[u8]) -> String {
    match locale_encoding() {
        Some(encoding) => encoding.decode_without_bom_handling(raw).0.into_owned(),
        None => String::from_utf8_lossy(raw).into_owned(),
    }
}

#[cfg(not(windows))]
fn decode_with_locale(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).into_owned()
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn locale_encoding() -> Option<&'static encoding_rs::Encoding> {
    // SAFETY: GetACP takes no arguments and only reads process-wide settings.
    let code_page = unsafe { windows::Win32::Globalization::GetACP() };
    encoding_for_code_page(code_page)
}

#[cfg(windows)]
fn encoding_for_code_page(code_page: u32) -> Option<&'static encoding_rs::Encoding> {
    let label = match code_page {
        874 => "windows-874",
        932 => "shift_jis",
        936 => "gbk",
        949 => "euc-kr",
        950 => "big5",
        1250..=1258 => {
            return encoding_rs::Encoding::for_label(format!("windows-{code_page}").as_bytes());
        }
        _ => return None,
    };
    encoding_rs::Encoding::for_label(label.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_names_are_kept() {
        assert_eq!(decode_name("日本語.txt".as_bytes()), "日本語.txt");
    }

    #[cfg(not(windows))]
    #[test]
    fn other_encodings_are_replaced_outside_windows() {
        // "日本" in Shift_JIS.
        let decoded = decode_name(&[0x93, 0xfa, 0x96, 0x7b]);

        assert!(decoded.contains('\u{fffd}'));
    }

    #[cfg(windows)]
    #[test]
    fn code_pages_map_to_encodings() {
        assert_eq!(
            encoding_for_code_page(932).map(encoding_rs::Encoding::name),
            Some("Shift_JIS")
        );
        assert_eq!(
            encoding_for_code_page(1252).map(encoding_rs::Encoding::name),
            Some("windows-1252")
        );
        assert_eq!(encoding_for_code_page(65001), None);
        assert_eq!(
            encoding_rs::SHIFT_JIS
                .decode_without_bom_handling(&[0x93, 0xfa, 0x96, 0x7b])
                .0,
            "日本"
        );
    }

    #[test]
    fn names_split_at_both_separators_and_drop_empty_parts() {
        assert_eq!(split_name("a/b/"), ["a", "b"]);
        assert_eq!(split_name("a\\b\\c.txt"), ["a", "b", "c.txt"]);
        assert_eq!(split_name("/abs//x"), ["abs", "x"]);
        assert_eq!(split_name(""), Vec::<String>::new());
    }

    #[test]
    fn unsafe_components_are_refused() {
        assert!(check_component("a.txt"));
        assert!(check_component("..."));
        assert!(!check_component(".."));
        assert!(!check_component("."));
        assert!(!check_component(""));
        assert!(!check_component("a\0b"));
        assert_eq!(check_component("c:"), !cfg!(windows));
    }
}
