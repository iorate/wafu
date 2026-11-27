use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::System::Ole::{OleInitialize, OleUninitialize};

// Enters a single-threaded apartment for the scope if the thread is not in one yet.
pub(crate) struct ComScope {
    initialized: bool,
}

impl ComScope {
    #[allow(unsafe_code)]
    pub(crate) fn enter() -> Self {
        // SAFETY: the call only affects this thread's COM state and is balanced in `drop`.
        let result = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        // `S_FALSE` means another initialization is in force and still needs the
        // balancing call; `RPC_E_CHANGED_MODE` means the thread is in another
        // apartment, which the shell objects used here tolerate.
        Self {
            initialized: result.is_ok(),
        }
    }
}

impl Drop for ComScope {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: paired with the successful `CoInitializeEx` in `enter`.
            unsafe { CoUninitialize() };
        }
    }
}

// Enters OLE for the scope, which drag and drop needs on top of COM; an
// initialization already in force is kept and balanced.
pub(crate) struct OleScope {
    initialized: bool,
}

impl OleScope {
    #[allow(unsafe_code)]
    pub(crate) fn enter() -> Self {
        // SAFETY: the call only affects this thread's OLE state and is balanced in `drop`.
        let result = unsafe { OleInitialize(None) };
        Self {
            initialized: result.is_ok(),
        }
    }
}

impl Drop for OleScope {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: paired with the successful `OleInitialize` in `enter`.
            unsafe { OleUninitialize() };
        }
    }
}
