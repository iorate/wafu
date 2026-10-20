#![windows_subsystem = "windows"]

mod cli;
mod keys;
mod ui;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use wafu_core::{CONFIG_TEMPLATE, Msg, Os, Start};
use wafu_io::Paths;

use crate::cli::Cli;
use crate::ui::Ui;

slint::include_modules!();

fn main() -> ExitCode {
    let cli = Cli::parse();
    let home = home_dir();
    let config_override = cli.config.map(absolute);
    let Some(config_path) = config_override.clone().or_else(wafu_io::config_path) else {
        eprintln!("wafu: cannot find the configuration directory; pass --config");
        return ExitCode::FAILURE;
    };
    let Some(state_path) = wafu_io::state_path() else {
        eprintln!("wafu: cannot find the state directory");
        return ExitCode::FAILURE;
    };
    let config = wafu_io::read_or_create(&config_path, CONFIG_TEMPLATE);
    let state = match &cli.inherit {
        Some(handoff) => wafu_io::read_handoff(handoff),
        None => wafu_io::read_state(&state_path),
    };
    let window = state
        .as_ref()
        .ok()
        .and_then(|state| state.as_ref())
        .and_then(|state| state.window.clone());

    let app_window = match AppWindow::new() {
        Ok(window) => window,
        Err(error) => {
            eprintln!("wafu: {error}");
            return ExitCode::FAILURE;
        }
    };
    let ui = Ui::install(
        app_window,
        Paths {
            state: state_path,
            config_override,
        },
    );
    ui.restore_window(window.as_ref());
    ui.dispatch(Msg::Start(Box::new(Start {
        left: cli.left.map(absolute),
        right: cli.right.map(absolute),
        home,
        os: os(),
        config_path,
        config,
        state,
    })));
    match ui.window().run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("wafu: {error}");
            ExitCode::FAILURE
        }
    }
}

fn os() -> Os {
    if cfg!(target_os = "windows") {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::MacOs
    } else {
        Os::Linux
    }
}

fn home_dir() -> PathBuf {
    std::env::home_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn absolute(path: PathBuf) -> PathBuf {
    std::path::absolute(&path).unwrap_or(path)
}
