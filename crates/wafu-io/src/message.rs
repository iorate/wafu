use windows::Win32::UI::WindowsAndMessaging::{MB_OK, MessageBoxW};
use windows::core::HSTRING;

/// Shows `text` in a message box titled `title` and waits for it to be closed.
/// This stands in for the console, which a GUI executable does not have.
#[allow(unsafe_code)]
pub fn show_message(title: &str, text: &str) {
    let title = HSTRING::from(title);
    let text = HSTRING::from(text);
    // SAFETY: both strings outlive the call, and a null window handle is allowed.
    unsafe {
        MessageBoxW(None, &text, &title, MB_OK);
    }
}
