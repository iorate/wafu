use std::path::PathBuf;

use clap::Parser;

/// A keyboard-driven dual-pane file manager.
#[derive(Debug, Parser)]
#[command(version, about)]
pub struct Cli {
    /// The path of the left pane.
    pub left: Option<PathBuf>,

    /// The path of the right pane.
    pub right: Option<PathBuf>,

    /// The configuration file to use instead of the default one.
    #[arg(long, value_name = "PATH")]
    pub config: Option<PathBuf>,

    // A state file written by the instance that started this one.
    #[arg(long, value_name = "PATH", hide = true)]
    pub inherit: Option<PathBuf>,
}
