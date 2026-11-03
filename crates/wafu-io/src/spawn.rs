use std::ffi::OsStr;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;

pub(crate) fn spawn(
    program: impl AsRef<OsStr>,
    args: &[impl AsRef<OsStr>],
    cwd: Option<&Path>,
) -> std::io::Result<Child> {
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
    command.spawn()
}

// Reaping the child keeps it from lingering as a zombie on Unix. The child
// has started whether or not the thread can be created, so that is not an
// error.
pub(crate) fn wait_in_background(
    mut child: Child,
    on_exit: impl FnOnce(ExitStatus) + Send + 'static,
) {
    let _ = thread::Builder::new()
        .name("wafu-io-reaper".to_owned())
        .spawn(move || {
            if let Ok(status) = child.wait() {
                on_exit(status);
            }
        });
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
    let child = spawn(exe, &args, None)?;
    wait_in_background(child, |_| {});
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_program_that_exists_starts() {
        let dir = TempDir::new().unwrap();
        let (program, args) = silent_program();

        let mut child = spawn(program, args, Some(dir.path())).unwrap();

        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn a_missing_program_or_directory_is_an_error() {
        let dir = TempDir::new().unwrap();
        let (program, args) = silent_program();

        assert!(spawn("wafu-no-such-program", &["x"], Some(dir.path())).is_err());
        assert!(spawn(program, args, Some(&dir.path().join("gone"))).is_err());
    }

    #[test]
    fn the_exit_status_is_given_to_the_callback() {
        let (program, args) = failing_program();
        let child = spawn(program, args, None).unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();

        wait_in_background(child, move |status| {
            let _ = sender.send(status);
        });

        let status = receiver
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        assert_eq!(status.code(), Some(1));
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

    fn failing_program() -> (&'static str, &'static [&'static str]) {
        if cfg!(windows) {
            ("cmd", &["/c", "exit", "1"])
        } else {
            ("false", &[])
        }
    }
}
