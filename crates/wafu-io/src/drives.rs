/// The letters of the local drives, in order. Network drives are left out.
/// Always empty except on Windows.
#[cfg(windows)]
#[must_use]
// The two calls are the only way to ask Windows about drives.
#[allow(unsafe_code)]
pub fn drives() -> Vec<char> {
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows::core::PCWSTR;

    // The value `GetDriveTypeW` documents for network drives; the constant
    // itself lives in a module that is not worth a whole feature.
    const DRIVE_REMOTE: u32 = 4;

    // SAFETY: the function takes no arguments and only reads system state.
    let mask = unsafe { GetLogicalDrives() };
    (0..26u8)
        .filter(|bit| mask & (1 << bit) != 0)
        .map(|bit| char::from(b'A' + bit))
        .filter(|letter| {
            let root: Vec<u16> = format!("{letter}:\\")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            // SAFETY: `root` is a NUL-terminated UTF-16 string that outlives the call.
            let kind = unsafe { GetDriveTypeW(PCWSTR(root.as_ptr())) };
            kind != DRIVE_REMOTE
        })
        .collect()
}

#[cfg(not(windows))]
#[must_use]
pub fn drives() -> Vec<char> {
    Vec::new()
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn the_system_drive_is_listed_in_order() {
        let letters = drives();

        assert!(letters.contains(&'C'));
        assert!(letters.iter().all(char::is_ascii_uppercase));
        assert!(letters.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
