use windows::Win32::Foundation::RPC_E_CHANGED_MODE;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};

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
            initialized: result.is_ok() && result != RPC_E_CHANGED_MODE,
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
