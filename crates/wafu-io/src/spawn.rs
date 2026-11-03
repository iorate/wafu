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
    // On Unix the child's output would otherwise land in the terminal that
    // started wafu. On Windows the streams are left alone and no creation
    // flag is set, so that a console program gets a console of its own, as
    // when started from Explorer. Standard error is never piped: descendants
    // inheriting the pipe would keep it open after the child has ended.
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
