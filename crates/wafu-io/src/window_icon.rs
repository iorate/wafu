use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::{
    ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED, LoadImageW, SM_CXSMICON, SM_CYSMICON,
    SendMessageW, WM_SETICON,
};
use windows::core::PCWSTR;

// The id the build gives the icon of the executable.
const ICON_RESOURCE: u16 = 1;

/// Gives the window the icon of the executable at the size its title bar
/// draws, so that the small image of the icon file is shown instead of a
/// large one scaled down. `window` is the HWND.
#[allow(unsafe_code)]
pub fn set_small_icon(window: isize) -> windows::core::Result<()> {
    let hwnd = HWND(window as *mut _);
    // SAFETY: the handle is a window of this process; the metrics take no
    // pointers.
    let (width, height) = unsafe {
        let dpi = GetDpiForWindow(hwnd);
        (
            GetSystemMetricsForDpi(SM_CXSMICON, dpi),
            GetSystemMetricsForDpi(SM_CYSMICON, dpi),
        )
    };
    // SAFETY: a null name asks for the handle of the executable itself.
    let module = unsafe { GetModuleHandleW(None) }?;
    // SAFETY: an integer in the name is a resource id, as `MAKEINTRESOURCE`
    // makes one; a shared icon is owned by the system and is not destroyed.
    let icon = unsafe {
        LoadImageW(
            Some(module.into()),
            PCWSTR(ICON_RESOURCE as usize as *const u16),
            IMAGE_ICON,
            width,
            height,
            LR_DEFAULTCOLOR | LR_SHARED,
        )
    }?;
    // SAFETY: the message takes the icon handle as its LPARAM; the previous
    // icon it returns belongs to whoever set it.
    unsafe {
        SendMessageW(
            hwnd,
            WM_SETICON,
            Some(WPARAM(ICON_SMALL as usize)),
            Some(LPARAM(icon.0 as isize)),
        );
    }
    Ok(())
}
