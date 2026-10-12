#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use wafu_core::{App, Effect, Msg};

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let window = AppWindow::new()?;
    let app = Rc::new(RefCell::new(App::new()));
    let _runtime = wafu_io::Runtime::new();

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    dispatch(
        &app,
        &window,
        Msg::Start {
            left: cwd.clone(),
            right: cwd,
        },
    );

    window.run()
}

fn dispatch(app: &Rc<RefCell<App>>, window: &AppWindow, msg: Msg) {
    let effects = app.borrow_mut().update(msg);
    for effect in effects {
        match effect {
            Effect::SetTitle(title) => window.set_window_title(title.into()),
        }
    }
}
