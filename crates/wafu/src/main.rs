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
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => return report_cli_error(&error),
    };
    #[cfg(unix)]
    if cli.print_env {
        wafu_io::print_env();
        return ExitCode::SUCCESS;
    }
    let home = home_dir();
    let config_override = cli.config.map(absolute);
    let Some(config_path) = config_override.clone().or_else(wafu_io::config_path) else {
        return report_startup_error("cannot find the configuration directory; pass --config");
    };
    let Some(state_path) = wafu_io::state_path() else {
        return report_startup_error("cannot find the state directory");
    };
    // Read before the window exists: its colors and fonts come from them.
    let config = wafu_io::read_or_create(&config_path, CONFIG_TEMPLATE);
    let inherited = cli.inherit.is_some();
    let (state, read_from) = match &cli.inherit {
        Some(handoff) => (wafu_io::read_handoff(handoff), handoff.clone()),
        None => (wafu_io::read_state(&state_path), state_path.clone()),
    };
    let window = state
        .as_ref()
        .ok()
        .and_then(|state| state.as_ref())
        .and_then(|state| state.window.clone());

    let app_window = match AppWindow::new() {
        Ok(window) => window,
        Err(error) => return report_startup_error(error),
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
        elevated: wafu_io::is_elevated(),
        config_path,
        config,
        state,
        state_path: read_from,
        inherited,
    })));
    match ui.window().run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => report_startup_error(error),
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

// A GUI executable on Windows has no console, so what clap would print
// (help, the version, or an argument error) goes to a message box.
#[cfg(windows)]
fn report_cli_error(error: &clap::Error) -> ExitCode {
    use clap::error::ErrorKind;

    wafu_io::show_message("wafu", &error.to_string());
    match error.kind() {
        ErrorKind::DisplayHelp | ErrorKind::DisplayVersion => ExitCode::SUCCESS,
        _ => ExitCode::from(2),
    }
}

#[cfg(not(windows))]
fn report_cli_error(error: &clap::Error) -> ExitCode {
    error.exit()
}

#[cfg(windows)]
fn report_startup_error(message: impl std::fmt::Display) -> ExitCode {
    wafu_io::show_message("wafu", &message.to_string());
    ExitCode::FAILURE
}

#[cfg(not(windows))]
fn report_startup_error(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("wafu: {message}");
    ExitCode::FAILURE
}
