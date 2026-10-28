use std::io;
use std::path::Path;

// The shell does the move so that it can warn when the trash cannot take the
// item and let the user choose between destroying it and keeping it; the
// `trash` crate suppresses that warning and destroys the item.
#[cfg(windows)]
pub(crate) fn trash(path: &Path, owner_window: Option<isize>) -> io::Result<()> {
    let _com = crate::com::ComScope::enter();
    shell_delete(path, owner_window).map_err(|error| {
        if error.code() == cancelled() {
            io::Error::other("cancelled in the dialog")
        } else {
            io::Error::other(error.to_string())
        }
    })
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn shell_delete(path: &Path, owner_window: Option<isize>) -> windows::core::Result<()> {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CLSCTX_ALL, CoCreateInstance};
    use windows::Win32::UI::Shell::{
        FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FOF_WANTNUKEWARNING,
        FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName,
    };
    use windows::core::HSTRING;

    // SAFETY: the class id is a documented constant and the thread is in an apartment.
    let operation: IFileOperation = unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_ALL) }?;
    // Progress and errors are shown by wafu, and moving to the trash needs no
    // confirmation; only the warning that an item would be destroyed stays.
    let flags =
        FOF_NOCONFIRMATION | FOF_ALLOWUNDO | FOF_WANTNUKEWARNING | FOF_SILENT | FOF_NOERRORUI;
    // SAFETY: a plain call on the object created above.
    unsafe { operation.SetOperationFlags(flags) }?;
    if let Some(window) = owner_window {
        // SAFETY: `window` is the handle of the application window, which outlives the operation.
        unsafe { operation.SetOwnerWindow(HWND(window as *mut _)) }?;
    }
    let name = HSTRING::from(path.as_os_str());
    // SAFETY: `name` is NUL-terminated and outlives the call; a bind context is optional.
    let item: IShellItem = unsafe { SHCreateItemFromParsingName(&name, None) }?;
    // SAFETY: `item` is a live shell item; a progress sink is optional.
    unsafe { operation.DeleteItem(&item, None) }?;
    // SAFETY: the operation is fully set up; the call blocks while the shell
    // works and shows its dialogs on this thread.
    unsafe { operation.PerformOperations() }?;
    // SAFETY: a query on the finished operation.
    if unsafe { operation.GetAnyOperationsAborted() }?.as_bool() {
        return Err(windows::core::Error::from_hresult(cancelled()));
    }
    Ok(())
}

#[cfg(windows)]
fn cancelled() -> windows::core::HRESULT {
    use windows::Win32::Foundation::ERROR_CANCELLED;

    windows::core::HRESULT::from_win32(ERROR_CANCELLED.0)
}

#[cfg(not(windows))]
pub(crate) fn trash(path: &Path, _owner_window: Option<isize>) -> io::Result<()> {
    trash_context()
        .delete(path)
        .map_err(|error| io::Error::other(error.to_string()))
}

// The default on macOS drives the Finder through AppleScript, which asks for
// permission to automate it and refuses symbolic links; NSFileManager does
// neither, at the cost of "Put Back" in the Finder.
#[cfg(target_os = "macos")]
fn trash_context() -> trash::TrashContext {
    use trash::macos::{DeleteMethod, TrashContextExtMacos};

    let mut context = trash::TrashContext::default();
    context.set_delete_method(DeleteMethod::NsFileManager);
    context
}

#[cfg(not(any(windows, target_os = "macos")))]
fn trash_context() -> trash::TrashContext {
    trash::TrashContext::default()
}
