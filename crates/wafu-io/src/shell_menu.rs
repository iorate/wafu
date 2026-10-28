use std::path::Path;

use thiserror::Error;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    BHID_SFUIObject, CMF_NORMAL, CMIC_MASK_PTINVOKE, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX,
    DefSubclassProc, IContextMenu, IContextMenu2, IContextMenu3, ILFree, IShellItemArray,
    RemoveWindowSubclass, SEE_MASK_UNICODE, SHCreateShellItemArrayFromIDLists, SHParseDisplayName,
    SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, HMENU, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenuEx, WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};
use windows::core::{HSTRING, Interface, PCSTR, PCWSTR};

use crate::com::ComScope;

/// Why the shell menu could not be shown.
#[derive(Debug, Error)]
pub enum ShellMenuError {
    #[error("no items")]
    Empty,
    #[error("{0}")]
    Windows(#[from] windows::core::Error),
}

// The command ids the menu may use; the shell numbers its items from the first.
const FIRST_COMMAND: u32 = 1;
const LAST_COMMAND: u32 = 0x7FFF;
// Identifies the subclass; any value works as long as it is unique on the window.
const SUBCLASS_ID: usize = 0x7761_6675;

/// Shows the shell's context menu for the paths and runs what the user picks.
/// `window` is the HWND of the owning window; `x` and `y` are in physical
/// pixels of its client area. Blocks until the menu closes.
// Everything here is Win32; the menu runs its own message loop on this thread.
#[allow(unsafe_code)]
pub fn shell_menu(
    window: isize,
    paths: &[impl AsRef<Path>],
    x: i32,
    y: i32,
) -> Result<(), ShellMenuError> {
    if paths.is_empty() {
        return Err(ShellMenuError::Empty);
    }
    let hwnd = HWND(window as *mut _);
    let _com = ComScope::enter();
    let menu = context_menu(paths)?;
    // SAFETY: the function takes no arguments and returns an owned menu handle.
    let hmenu = unsafe { CreatePopupMenu() }?;
    let result = track(hwnd, &menu, hmenu, paths[0].as_ref(), x, y);
    // SAFETY: `hmenu` was created above and is no longer tracked.
    let _ = unsafe { DestroyMenu(hmenu) };
    result
}

// The context menu handler of the paths, which all lie in the same directory.
#[allow(unsafe_code)]
fn context_menu(paths: &[impl AsRef<Path>]) -> Result<IContextMenu, ShellMenuError> {
    let mut pidls: Vec<*const ITEMIDLIST> = Vec::with_capacity(paths.len());
    let parse = |path: &Path| -> windows::core::Result<*const ITEMIDLIST> {
        let name = HSTRING::from(path.as_os_str());
        let mut pidl = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated and outlives the call; `pidl` receives
        // an item list that is freed below.
        unsafe { SHParseDisplayName(&name, None, &raw mut pidl, 0, None) }?;
        Ok(pidl.cast_const())
    };
    let array = (|| {
        for path in paths {
            pidls.push(parse(path.as_ref())?);
        }
        // SAFETY: every pointer in `pidls` is a valid item list.
        let array: IShellItemArray = unsafe { SHCreateShellItemArrayFromIDLists(&pidls) }?;
        // SAFETY: a bind context is optional and the handler id is a documented constant.
        unsafe { array.BindToHandler::<_, IContextMenu>(None, &BHID_SFUIObject) }
    })();
    for pidl in pidls {
        // SAFETY: each item list came from `SHParseDisplayName` and is used no more.
        unsafe { ILFree(Some(pidl)) };
    }
    Ok(array?)
}

// Fills the menu, shows it, and runs the chosen command.
#[allow(unsafe_code)]
fn track(
    hwnd: HWND,
    menu: &IContextMenu,
    hmenu: HMENU,
    first: &Path,
    x: i32,
    y: i32,
) -> Result<(), ShellMenuError> {
    // SAFETY: `hmenu` is an empty popup menu owned by the caller.
    unsafe { menu.QueryContextMenu(hmenu, 0, FIRST_COMMAND, LAST_COMMAND, CMF_NORMAL) }.ok()?;

    // Submenus such as "Send to" are filled in reply to the menu messages,
    // which the shell wants forwarded from the owning window.
    let handler = menu
        .cast::<IContextMenu3>()
        .map(Handler::V3)
        .or_else(|_| menu.cast::<IContextMenu2>().map(Handler::V2))
        .ok();
    let subclassed = handler.as_ref().is_some_and(|handler| {
        // SAFETY: `handler` lives until the subclass is removed below, on this thread.
        unsafe {
            SetWindowSubclass(
                hwnd,
                Some(forward_menu_messages),
                SUBCLASS_ID,
                std::ptr::from_ref(handler) as usize,
            )
        }
        .as_bool()
    });

    let mut point = POINT { x, y };
    // SAFETY: `point` is a valid, writable point and `hwnd` is the caller's window.
    let _ = unsafe { ClientToScreen(hwnd, &raw mut point) };
    let flags = TPM_RETURNCMD | TPM_RIGHTBUTTON;
    // SAFETY: the menu and the window are valid for the duration of the call,
    // which runs the menu's message loop on this thread.
    let chosen = unsafe { TrackPopupMenuEx(hmenu, flags.0, point.x, point.y, hwnd, None) }.0;

    if subclassed {
        // SAFETY: removes the subclass installed above before `handler` is dropped.
        let _ = unsafe { RemoveWindowSubclass(hwnd, Some(forward_menu_messages), SUBCLASS_ID) };
    }
    drop(handler);

    let Ok(chosen) = u32::try_from(chosen) else {
        return Ok(());
    };
    if chosen < FIRST_COMMAND {
        return Ok(());
    }
    let verb = (chosen - FIRST_COMMAND) as usize;
    let directory = HSTRING::from(first.parent().unwrap_or(first).as_os_str());
    let info = CMINVOKECOMMANDINFOEX {
        cbSize: u32::try_from(size_of::<CMINVOKECOMMANDINFOEX>()).unwrap_or(0),
        fMask: SEE_MASK_UNICODE | CMIC_MASK_PTINVOKE,
        hwnd,
        lpVerb: PCSTR(verb as *const u8),
        lpVerbW: PCWSTR(verb as *const u16),
        lpDirectoryW: PCWSTR(directory.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ptInvoke: point,
        ..Default::default()
    };
    // SAFETY: `info` is a fully initialized extended structure, which the
    // interface accepts in place of the basic one, and `directory` outlives the call.
    unsafe { menu.InvokeCommand(std::ptr::from_ref(&info).cast::<CMINVOKECOMMANDINFO>()) }?;
    Ok(())
}

enum Handler {
    V3(IContextMenu3),
    V2(IContextMenu2),
}

#[allow(unsafe_code)]
unsafe extern "system" fn forward_menu_messages(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    refdata: usize,
) -> LRESULT {
    if matches!(
        msg,
        WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR
    ) {
        // SAFETY: `refdata` is the `Handler` that `track` registered, which
        // outlives the subclass; the subclass runs on the registering thread.
        let handler = unsafe { &*(refdata as *const Handler) };
        let mut result = LRESULT(0);
        let forwarded = match handler {
            // SAFETY: the messages are forwarded unchanged from the window procedure.
            Handler::V3(menu) => {
                unsafe { menu.HandleMenuMsg2(msg, wparam, lparam, Some(&raw mut result)) }.is_ok()
            }
            // SAFETY: as above.
            Handler::V2(menu) => unsafe { menu.HandleMenuMsg(msg, wparam, lparam) }.is_ok(),
        };
        if forwarded {
            if matches!(handler, Handler::V2(_)) && matches!(msg, WM_DRAWITEM | WM_MEASUREITEM) {
                return LRESULT(1);
            }
            return result;
        }
    }
    // SAFETY: the standard continuation of a subclass chain.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}
