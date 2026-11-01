use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;

pub(crate) fn spawn(
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
    let _ = thread::Builder::new()
        .name("wafu-io-reaper".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

pub(crate) fn spawn_new_window(handoff: &Path, config: Option<&Path>) -> std::io::Result<()> {
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
        let (program, args) = silent_program();

        spawn(program, args, Some(dir.path())).unwrap();
    }

    #[test]
    fn a_missing_program_or_directory_is_an_error() {
        let dir = TempDir::new().unwrap();
        let (program, args) = silent_program();

        assert!(spawn("wafu-no-such-program", &["x"], Some(dir.path())).is_err());
        assert!(spawn(program, args, Some(&dir.path().join("gone"))).is_err());
    }

    // The child inherits the test's console on Windows, so anything it printed
    // would end up in the test output.
    fn silent_program() -> (&'static str, &'static [&'static str]) {
        if cfg!(windows) {
            ("cmd", &["/c", "exit"])
        } else {
            ("true", &[])
        }
    }
}
