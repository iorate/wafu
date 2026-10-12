#![windows_subsystem = "windows"]

mod keys;
mod ui;

use std::path::PathBuf;

use wafu_core::Msg;

use crate::ui::Ui;

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let mut args = std::env::args_os().skip(1);
    let home = home_dir();
    let left = args.next().map_or_else(|| home.clone(), resolve);
    let right = args.next().map_or(home, resolve);

    let ui = Ui::install(AppWindow::new()?);
    ui.dispatch(Msg::Start { left, right });
    ui.window().run()
}

fn home_dir() -> PathBuf {
    std::env::home_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn resolve(arg: std::ffi::OsString) -> PathBuf {
    let path = PathBuf::from(arg);
    std::path::absolute(&path).unwrap_or(path)
}
