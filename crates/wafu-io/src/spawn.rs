use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;

pub(crate) fn spawn(
    program: impl AsRef<OsStr>,
    args: &[impl AsRef<OsStr>],
    cwd: Option<&Path>,
    env: &[(OsString, OsString)],
) -> std::io::Result<Child> {
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(env.iter().map(|(key, value)| (key, value)));
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

pub(crate) fn spawn_new_window(
    handoff: &Path,
    config: Option<&Path>,
    as_admin: bool,
    env: &[(OsString, OsString)],
) -> std::io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut args = vec![
        OsStr::new("--inherit").to_owned(),
        handoff.as_os_str().to_owned(),
    ];
    if let Some(config) = config {
        args.push(OsStr::new("--config").to_owned());
        args.push(config.as_os_str().to_owned());
    }
    if as_admin {
        return spawn_elevated(&exe, &args);
    }
    let child = spawn(exe, &args, None, env)?;
    wait_in_background(child, |_| {});
    Ok(())
}

// The elevated process is started by the shell, which asks through UAC; a
// refusal comes back as `ERROR_CANCELLED`. Error dialogs of the shell are
// suppressed so that every failure reaches the log instead.
#[cfg(windows)]
#[allow(unsafe_code)]
fn spawn_elevated(exe: &Path, args: &[OsString]) -> std::io::Result<()> {
    use windows::Win32::UI::Shell::{
        SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};

    use crate::com::ComScope;

    let file = HSTRING::from(exe.as_os_str());
    let parameters = command_line(args);
    let _com = ComScope::enter();
    let mut info = SHELLEXECUTEINFOW {
        // The struct is a few dozen bytes.
        #[allow(clippy::cast_possible_truncation)]
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
        lpVerb: w!("runas"),
        lpFile: windows::core::PCWSTR(file.as_ptr()),
        lpParameters: windows::core::PCWSTR(parameters.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` is fully initialized, and the strings it points to
    // outlive the call.
    unsafe { ShellExecuteExW(&raw mut info) }.map_err(std::io::Error::other)
}

#[cfg(not(windows))]
fn spawn_elevated(_exe: &Path, _args: &[OsString]) -> std::io::Result<()> {
    Err(std::io::Error::other(
        "starting a window as administrator is available only on Windows",
    ))
}

// The arguments are paths, which cannot contain a double quote on Windows, so
// quoting only has to cover spaces.
#[cfg(windows)]
fn command_line(args: &[OsString]) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;

    let mut line = Vec::new();
    for (index, arg) in args.iter().enumerate() {
        if index > 0 {
            line.push(u16::from(b' '));
        }
        let quoted = arg
            .encode_wide()
            .any(|c| c == u16::from(b' ') || c == u16::from(b'\t'));
        if quoted {
            line.push(u16::from(b'"'));
        }
        line.extend(arg.encode_wide());
        if quoted {
            line.push(u16::from(b'"'));
        }
    }
    line.push(0);
    line
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn a_missing_program_or_directory_is_an_error() {
        let dir = TempDir::new().unwrap();
        let (program, args) = silent_program();

        assert!(spawn("wafu-no-such-program", &["x"], Some(dir.path()), &[]).is_err());
        assert!(spawn(program, args, Some(&dir.path().join("gone")), &[]).is_err());
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
