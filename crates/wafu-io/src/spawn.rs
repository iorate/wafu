use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

/// Starts a program directly, without a shell, and does not wait for it.
pub fn spawn(program: &str, args: &[impl AsRef<OsStr>], cwd: &Path) -> std::io::Result<()> {
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    hide_console(&mut command);
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
    let cwd = std::env::current_dir()?;
    let mut args = vec![
        OsStr::new("--inherit").to_owned(),
        handoff.as_os_str().to_owned(),
    ];
    if let Some(config) = config {
        args.push(OsStr::new("--config").to_owned());
        args.push(config.as_os_str().to_owned());
    }
    spawn(&exe.to_string_lossy(), &args, &cwd)
}

#[cfg(windows)]
fn hide_console(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
fn hide_console(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_program_that_exists_starts() {
        let dir = TempDir::new().unwrap();
        let exe = std::env::current_exe().unwrap();

        spawn(&exe.to_string_lossy(), &["--help"], dir.path()).unwrap();
    }

    #[test]
    fn a_missing_program_or_directory_is_an_error() {
        let dir = TempDir::new().unwrap();
        let exe = std::env::current_exe().unwrap();

        assert!(spawn("wafu-no-such-program", &["x"], dir.path()).is_err());
        assert!(
            spawn(
                &exe.to_string_lossy(),
                &["--help"],
                &dir.path().join("gone")
            )
            .is_err()
        );
    }
}
