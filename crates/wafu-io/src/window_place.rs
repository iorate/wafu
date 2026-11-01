use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromRect,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowInfo, GetWindowPlacement, WINDOWINFO, WINDOWPLACEMENT, WS_EX_TOOLWINDOW, WS_MAXIMIZE,
    WS_MINIMIZE,
};

/// A place of a window in physical pixels: the screen position of the
/// top-left corner of its frame and the size inside the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowPlace {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Returns where the window goes when it is restored from being minimized or
/// maximized. `window` is the HWND.
#[allow(unsafe_code)]
pub fn normal_window_place(window: isize) -> windows::core::Result<WindowPlace> {
    let hwnd = HWND(window as *mut _);
    let mut placement = WINDOWPLACEMENT {
        length: u32::try_from(size_of::<WINDOWPLACEMENT>()).unwrap_or(0),
        ..Default::default()
    };
    // SAFETY: the handle is a window of this process, and the structure is
    // writable with its length set as the call requires.
    unsafe { GetWindowPlacement(hwnd, &raw mut placement) }?;
    let mut info = WINDOWINFO {
        cbSize: u32::try_from(size_of::<WINDOWINFO>()).unwrap_or(0),
        ..Default::default()
    };
    // SAFETY: as above, with the size of the structure set.
    unsafe { GetWindowInfo(hwnd, &raw mut info) }?;
    let rect = placement.rcNormalPosition;
    let (mut x, mut y) = (rect.left, rect.top);
    if !info.dwExStyle.contains(WS_EX_TOOLWINDOW) {
        // SAFETY: the rectangle is read only during the call.
        let monitor = unsafe { MonitorFromRect(&raw const rect, MONITOR_DEFAULTTONEAREST) };
        let mut monitor_info = MONITORINFO {
            cbSize: u32::try_from(size_of::<MONITORINFO>()).unwrap_or(0),
            ..Default::default()
        };
        // SAFETY: the nearest monitor always exists, and the structure is
        // writable with its size set.
        unsafe { GetMonitorInfoW(monitor, &raw mut monitor_info) }.ok()?;
        x += monitor_info.rcWork.left - monitor_info.rcMonitor.left;
        y += monitor_info.rcWork.top - monitor_info.rcMonitor.top;
    }
    let mut frame = RECT::default();
    // SAFETY: the handle is a window of this process, and the rectangle is
    // writable.
    unsafe {
        let dpi = GetDpiForWindow(hwnd);
        AdjustWindowRectExForDpi(
            &raw mut frame,
            info.dwStyle & !(WS_MINIMIZE | WS_MAXIMIZE),
            false,
            info.dwExStyle,
            dpi,
        )
    }?;
    let inner = |outer: i32, frame: i32| u32::try_from(outer - frame).unwrap_or(0);
    Ok(WindowPlace {
        x,
        y,
        width: inner(rect.right - rect.left, frame.right - frame.left),
        height: inner(rect.bottom - rect.top, frame.bottom - frame.top),
    })
}
