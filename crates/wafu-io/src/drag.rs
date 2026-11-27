use std::path::Path;

#[cfg(target_os = "macos")]
use objc2::rc::Retained;
#[cfg(target_os = "macos")]
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
#[cfg(target_os = "macos")]
use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send};
#[cfg(target_os = "macos")]
use objc2_app_kit::{
    NSApplication, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession,
    NSDraggingSource, NSEvent, NSEventType, NSView, NSWorkspace,
};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSPoint, NSProcessInfo, NSRect, NSURL};
#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
#[cfg(windows)]
use windows::Win32::Foundation::HWND;
#[cfg(windows)]
use windows::Win32::System::Com::IDataObject;
#[cfg(windows)]
use windows::Win32::System::Ole::DROPEFFECT_COPY;
#[cfg(windows)]
use windows::Win32::UI::Shell::{BHID_DataObject, SHDoDragDrop};

#[cfg(windows)]
use crate::com::OleScope;
#[cfg(windows)]
use crate::shell_menu::bind_items;

/// Why a drag could not start on macOS.
#[cfg(target_os = "macos")]
#[derive(Debug, thiserror::Error)]
pub enum DragError {
    #[error("not on the main thread")]
    NotMainThread,
    #[error("the window has no view")]
    NoView,
    #[error("no mouse event to start from")]
    NoMouseEvent,
    #[error("a path cannot be made into a file URL")]
    NotAFileUrl,
    #[error("no dragging session was started")]
    NotStarted,
}

/// Drags the paths, which all lie in the same directory, out of the window to
/// where the user drops them, letting the target only copy them. `window` is
/// the HWND of the window the drag starts from. Blocks until the drop or the
/// cancellation, neither of which is an error.
// The shell's drag rather than `DoDragDrop`: given no drop source, it makes
// its own, which also shows the drag image, so wafu implements no COM object.
#[cfg(windows)]
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

/// Drags the paths out of the window to where the user drops them, letting
/// the target only copy them. `window` is the window the drag starts from,
/// and the call has to be made on the main thread while the window handles
/// the press or the move of the left button that starts the drag. Returns
/// once the drag has started; it then runs in the event loop, and its end is
/// not reported.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
pub fn drag_files(
    window: &impl HasWindowHandle,
    paths: &[impl AsRef<Path>],
) -> Result<(), DragError> {
    let mtm = MainThreadMarker::new().ok_or(DragError::NotMainThread)?;
    let window = window.window_handle().map_err(|_| DragError::NoView)?;
    let RawWindowHandle::AppKit(handle) = window.as_raw() else {
        return Err(DragError::NoView);
    };
    // SAFETY: the handle points to the NSView of the window, which stays
    // alive while `window` borrows it, and this is the main thread.
    let view = unsafe { handle.ns_view.cast::<NSView>().as_ref() };
    let current = NSApplication::sharedApplication(mtm)
        .currentEvent()
        .filter(|event| {
            matches!(
                event.r#type(),
                NSEventType::LeftMouseDown | NSEventType::LeftMouseDragged
            )
        });
    // The move that starts the drag is not always the event AppKit has at
    // hand when the call is made. A drag of the left button is then made up
    // from where the pointer is, unless the button has been let go, in which
    // case the press was a click and nothing is dragged.
    let event = if let Some(event) = current {
        event
    } else {
        if NSEvent::pressedMouseButtons() & 1 == 0 {
            return Ok(());
        }
        let window = view.window().ok_or(DragError::NoView)?;
        NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                NSEventType::LeftMouseDragged,
                window.mouseLocationOutsideOfEventStream(),
                NSEvent::modifierFlags_class(),
                NSProcessInfo::processInfo().systemUptime(),
                window.windowNumber(),
                None,
                0,
                1,
                1.0,
            )
            .ok_or(DragError::NoMouseEvent)?
    };
    let pointer = view.convertPoint_fromView(event.locationInWindow(), None);
    let workspace = NSWorkspace::sharedWorkspace();
    let mut items = Vec::with_capacity(paths.len());
    for path in paths {
        let url = if path.as_ref().is_dir() {
            NSURL::from_directory_path(path)
        } else {
            NSURL::from_file_path(path)
        }
        .ok_or(DragError::NotAFileUrl)?;
        let file = url.path().ok_or(DragError::NotAFileUrl)?;
        let icon = workspace.iconForFile(&file);
        let item = NSDraggingItem::initWithPasteboardWriter(
            NSDraggingItem::alloc(),
            ProtocolObject::from_ref(&*url),
        );
        let size = icon.size();
        let origin = NSPoint::new(pointer.x - size.width / 2.0, pointer.y - size.height / 2.0);
        // SAFETY: an NSImage is what the image of a dragging item is drawn from.
        unsafe { item.setDraggingFrame_contents(NSRect::new(origin, size), Some(&icon)) };
        items.push(item);
    }
    let source = drag_source(mtm);
    let items = NSArray::from_retained_slice(&items);
    // Sent by hand because the binding declares that a session is always
    // returned, while AppKit documents nil for a session it cannot start.
    // SAFETY: the arguments are the array of dragging items, the mouse event
    // and the dragging source that the method takes.
    let session: Option<Retained<NSDraggingSession>> = unsafe {
        msg_send![
            view,
            beginDraggingSessionWithItems: &*items,
            event: &*event,
            source: &*source,
        ]
    };
    session.map(drop).ok_or(DragError::NotStarted)
}

// One source for every drag, kept for the life of the thread: AppKit does not
// document that the session keeps its source alive, and the source has no
// state of its own.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn drag_source(mtm: MainThreadMarker) -> Retained<ProtocolObject<dyn NSDraggingSource>> {
    define_class!(
        // SAFETY: NSObject has no requirements on its subclasses, and the
        // class has no `Drop`.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "WafuDragSource"]
        struct DragSource;

        unsafe impl NSObjectProtocol for DragSource {}

        // SAFETY: the method has the signature that the protocol declares.
        unsafe impl NSDraggingSource for DragSource {
            #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
            fn operation_mask(
                &self,
                _session: &NSDraggingSession,
                _context: NSDraggingContext,
            ) -> NSDragOperation {
                NSDragOperation::Copy
            }
        }
    );

    thread_local! {
        static SOURCE: std::cell::OnceCell<Retained<ProtocolObject<dyn NSDraggingSource>>> =
            const { std::cell::OnceCell::new() };
    }
    SOURCE.with(|source| {
        source
            .get_or_init(|| {
                // SAFETY: `init` of NSObject takes no arguments and returns
                // the initialized object.
                let source: Retained<DragSource> =
                    unsafe { msg_send![super(DragSource::alloc(mtm).set_ivars(())), init] };
                ProtocolObject::from_retained(source)
            })
            .clone()
    })
}
