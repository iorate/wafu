use std::path::Path;

use windows::Win32::Foundation::HWND;
use windows::Win32::System::Com::IDataObject;
use windows::Win32::System::Ole::DROPEFFECT_COPY;
use windows::Win32::UI::Shell::{BHID_DataObject, SHDoDragDrop};

use crate::com::OleScope;
use crate::shell_menu::bind_items;

/// Drags the paths, which all lie in the same directory, out of the window to
/// where the user drops them, letting the target only copy them. `window` is
/// the HWND of the window the drag starts from. Blocks until the drop or the
/// cancellation, neither of which is an error.
// The shell's drag rather than `DoDragDrop`: given no drop source, it makes
// its own, which also shows the drag image, so wafu implements no COM object.
#[allow(unsafe_code)]
pub fn drag_files(window: isize, paths: &[impl AsRef<Path>]) -> windows::core::Result<()> {
    let hwnd = HWND(window as *mut _);
    let _ole = OleScope::enter();
    let data: IDataObject = bind_items(paths, &BHID_DataObject)?;
    // SAFETY: `data` is a live data object and `hwnd` is the caller's window;
    // the call runs the drag's message loop on this thread.
    unsafe { SHDoDragDrop(Some(hwnd), &data, None, DROPEFFECT_COPY) }?;
    Ok(())
}
