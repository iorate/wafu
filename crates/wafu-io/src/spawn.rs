use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

/// Starts a program directly, without a shell, and does not wait for it.
pub fn spawn(
    program: impl AsRef<OsStr>,
    args: &[impl AsRef<OsStr>],
    cwd: Option<&Path>,
) -> std::io::Result<()> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    if !cfg!(windows) {
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
    }
    let mut child = command.spawn()?;
    // Reaping the child keeps it from lingering as a zombie on Unix.
    thread::Builder::new()
        .name("wafu-io-reaper".to_owned())
        .spawn(move || {
            let _ = child.wait();
        })?;
    Ok(())
}

/// Starts another instance of this program that takes over the state in the
/// handoff file.
pub fn spawn_new_window(handoff: &Path, config: Option<&Path>) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut args = vec![
        OsStr::new("--inherit").to_owned(),
        handoff.as_os_str().to_owned(),
    ];
    if let Some(config) = config {
        args.push(OsStr::new("--config").to_owned());
        args.push(config.as_os_str().to_owned());
    }
    spawn(exe, &args, None)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_program_that_exists_starts() {
        let dir = TempDir::new().unwrap();
        let exe = std::env::current_exe().unwrap();

        spawn(&exe, &["--help"], Some(dir.path())).unwrap();
    }

    #[test]
    fn a_missing_program_or_directory_is_an_error() {
        let dir = TempDir::new().unwrap();
        let exe = std::env::current_exe().unwrap();

        assert!(spawn("wafu-no-such-program", &["x"], Some(dir.path())).is_err());
        assert!(spawn(&exe, &["--help"], Some(&dir.path().join("gone"))).is_err());
    }
}
