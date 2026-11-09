use std::ffi::{OsStr, OsString};
use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::spawn::wait_in_background;

const TIMEOUT: Duration = Duration::from_secs(10);
const RESOLVING: &str = "WAFU_RESOLVING_ENVIRONMENT";
// Compiled in rather than passed on the command line: rc files that echo each
// command (a DEBUG trap, a hook setting the terminal title) would otherwise
// print a marker before the environment does.
const BEGIN: &[u8] = b"wafu-shell-env-begin-5c0e2b7a9d41f386";
const END: &[u8] = b"wafu-shell-env-end-a3f91d6c08e7b254";

/// Writes the environment of this process to standard output, in the form the
/// instance reading the login shell's environment expects.
pub fn print_env() {
    let mut output = BEGIN.to_vec();
    for (key, value) in std::env::vars_os() {
        output.extend_from_slice(key.as_bytes());
        output.push(b'=');
        output.extend_from_slice(value.as_bytes());
        output.push(0);
    }
    output.extend_from_slice(END);
    let mut stdout = io::stdout().lock();
    let _ = stdout.write_all(&output).and_then(|()| stdout.flush());
}

// zsh is the default login shell on macOS; finding the user's shell without
// `$SHELL` would take the passwd database, which only unsafe code can reach.
pub(crate) fn load_shell_env() -> io::Result<Vec<(OsString, OsString)>> {
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| OsString::from("/bin/zsh"));
    let shell = Path::new(&shell);
    // These shells do not take the command line `resolve` builds (PowerShell
    // needs `&` before a quoted path, Nushell needs `^`, and xonsh parses the
    // line as Python), so trying would fail on every start with an error no rc
    // file can fix. Programs keep the environment wafu was launched with.
    if matches!(
        shell.file_name().and_then(OsStr::to_str),
        Some("pwsh" | "powershell" | "pwsh-preview" | "powershell-preview" | "nu" | "xonsh")
    ) {
        return Ok(Vec::new());
    }
    resolve(shell, &std::env::current_exe()?, TIMEOUT)
}

// The shell runs as an interactive login shell so that both kinds of rc files
// run, `.zprofile` and `.zshrc` (`.bash_profile` and `.bashrc`). The result is
// taken as soon as it is complete, without waiting for the shell to exit or
// for the end of its output: a background process started by an rc file can
// keep the pipe open forever, and a shell may exit with an error that has
// nothing to do with the environment.
fn resolve(
    shell: &Path,
    printer: &Path,
    timeout: Duration,
) -> io::Result<Vec<(OsString, OsString)>> {
    let mut line = quote(printer.as_os_str());
    line.push(" --print-env");
    let mut command = Command::new(shell);
    // csh and tcsh reject the login flag together with `-c`.
    match shell.file_name().and_then(OsStr::to_str) {
        Some("csh" | "tcsh") => command.arg("-ic"),
        _ => command.args(["-i", "-l", "-c"]),
    };
    // An rc file that reads input gets the end of it instead of waiting.
    command
        .arg(line)
        .env(RESOLVING, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn()?;
    let mut stdout = child
        .stdout
        .take()
        .expect("standard output should be piped");
    let (sender, receiver) = mpsc::channel();
    let reader = thread::Builder::new()
        .name("wafu-io-shell-env-reader".to_owned())
        .spawn(move || {
            let _ = sender.send(read_until_end(&mut stdout));
        });
    let output = match reader {
        Ok(_) => receiver.recv_timeout(timeout).ok(),
        Err(error) => {
            let _ = child.kill();
            wait_in_background(child, |_| {});
            return Err(error);
        }
    };
    let Some(output) = output else {
        let _ = child.kill();
        wait_in_background(child, |_| {});
        return Err(io::Error::new(
            ErrorKind::TimedOut,
            format!(
                "the shell did not report its environment within {} seconds",
                timeout.as_secs()
            ),
        ));
    };
    wait_in_background(child, |_| {});
    parse(&output).ok_or_else(|| io::Error::other("the shell did not report its environment"))
}

fn quote(arg: &OsStr) -> OsString {
    let mut quoted = vec![b'\''];
    for &byte in arg.as_bytes() {
        if byte == b'\'' {
            quoted.extend_from_slice(b"'\\''");
        } else {
            quoted.push(byte);
        }
    }
    quoted.push(b'\'');
    OsString::from_vec(quoted)
}

fn read_until_end(reader: &mut impl Read) -> Vec<u8> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => {
                output.extend_from_slice(&buffer[..read]);
                if payload(&output).is_some() {
                    break;
                }
            }
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    output
}

// Whatever the rc files print may come before the begin marker and after the
// end marker.
fn payload(output: &[u8]) -> Option<&[u8]> {
    let start = find(output, BEGIN)? + BEGIN.len();
    let len = find(&output[start..], END)?;
    Some(&output[start..start + len])
}

fn parse(output: &[u8]) -> Option<Vec<(OsString, OsString)>> {
    let vars = payload(output)?
        .split(|&byte| byte == 0)
        .filter_map(|entry| {
            let equals = entry.iter().position(|&byte| byte == b'=')?;
            let key = OsStr::from_bytes(&entry[..equals]);
            (key != RESOLVING).then(|| {
                (
                    key.to_owned(),
                    OsStr::from_bytes(&entry[equals + 1..]).to_owned(),
                )
            })
        })
        .collect();
    Some(vars)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use tempfile::TempDir;

    use super::*;

    const SH: &str = "/bin/sh";

    #[test]
    fn parse_takes_the_variables_between_the_markers() {
        let mut output = b"Last login: today\n".to_vec();
        output.extend_from_slice(BEGIN);
        output.extend_from_slice(b"A=x=y\nz\0B=\0");
        output.extend_from_slice(END);
        output.extend_from_slice(b"bye\n");

        assert_eq!(
            parse(&output),
            Some(vec![
                (OsString::from("A"), OsString::from("x=y\nz")),
                (OsString::from("B"), OsString::new()),
            ])
        );
    }

    #[test]
    fn resolve_returns_what_the_printer_prints_without_the_marker_variable() {
        let dir = TempDir::new().unwrap();
        let printer = printer(&dir, "");

        let vars = resolve(Path::new(SH), &printer, Duration::from_secs(10)).unwrap();

        assert_eq!(
            vars,
            [(OsString::from("SEEN"), OsString::from("1"))],
            "{RESOLVING} should reach the shell and stay out of the result"
        );
    }

    #[test]
    fn resolve_does_not_wait_for_a_background_process_holding_the_output() {
        let dir = TempDir::new().unwrap();
        let printer = printer(&dir, "sleep 30 &\n");

        let vars = resolve(Path::new(SH), &printer, Duration::from_secs(5)).unwrap();

        assert_eq!(vars, [(OsString::from("SEEN"), OsString::from("1"))]);
    }

    #[test]
    fn resolve_gives_up_after_the_timeout() {
        let dir = TempDir::new().unwrap();
        let printer = script(&dir, "sleep 5\n");

        let error = resolve(Path::new(SH), &printer, Duration::from_millis(200)).unwrap_err();

        assert_eq!(error.kind(), ErrorKind::TimedOut);
    }

    // A script standing in for wafu that prints `SEEN` with the value of the
    // marker variable, the marker variable itself, and then runs `after`.
    fn printer(dir: &TempDir, after: &str) -> PathBuf {
        let begin = std::str::from_utf8(BEGIN).unwrap();
        let end = std::str::from_utf8(END).unwrap();
        script(
            dir,
            &format!(
                "echo noise\nprintf '%s' '{begin}'\nprintf 'SEEN=%s\\000' \"${RESOLVING}\"\nprintf '{RESOLVING}=1\\000'\nprintf '%s' '{end}'\n{after}"
            ),
        )
    }

    fn script(dir: &TempDir, body: &str) -> PathBuf {
        let path = dir.path().join("it's wafu");
        fs::write(&path, format!("#!/bin/sh\n{body}")).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}
