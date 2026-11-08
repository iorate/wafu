/// Whether the process runs with administrator rights (Windows). Always false
/// on the other systems.
#[cfg(windows)]
#[must_use]
#[allow(unsafe_code)]
pub fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    let mut token = HANDLE::default();
    // SAFETY: the pseudo handle of the current process needs no closing, and
    // `token` is writable.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &raw mut token) }.is_err() {
        return false;
    }
    let mut elevation = TOKEN_ELEVATION::default();
    let mut written = 0u32;
    // The struct is a single u32.
    #[allow(clippy::cast_possible_truncation)]
    let size = std::mem::size_of::<TOKEN_ELEVATION>() as u32;
    // SAFETY: the token is open, and `elevation` is writable and as large as
    // the length passed.
    let queried = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size,
            &raw mut written,
        )
    };
    // SAFETY: the token was opened above and is not used afterwards.
    let _ = unsafe { CloseHandle(token) };
    queried.is_ok() && elevation.TokenIsElevated != 0
}

/// Whether the process runs with administrator rights (Windows). Always false
/// on the other systems.
#[cfg(not(windows))]
#[must_use]
pub fn is_elevated() -> bool {
    false
}
