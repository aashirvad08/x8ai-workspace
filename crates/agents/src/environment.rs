//! The environment an agent would have if the user typed its name in their own
//! terminal.
//!
//! A GUI app inherits launchd's minimal environment, whose `PATH` lacks most
//! developer tools. Terminals fix that by starting an interactive login shell,
//! which reads the user's startup files (`.zprofile`, `.zshrc`, …). Agents are
//! started directly, without a shell, so the runtime asks the user's shell for
//! that environment once: it runs `$SHELL -l -i -c` with a fixed script that prints
//! the environment between two markers.
//!
//! The shell runs in the home directory, never in a workspace, so nothing a
//! repository provides (such as a `.envrc`) is read. It gets no input, and it is
//! killed with its whole process group if it does not finish in time.

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

/// How long the user's shell may take to start before the attempt is abandoned.
pub const RESOLVE_TIMEOUT: Duration = Duration::from_secs(10);

/// Output beyond this is not an environment, and reading stops.
const MAX_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

/// Carries the marker into the shell, so the script itself is a constant.
const MARKER_VAR: &str = "X8AI_ENV_MARKER";

/// Prints the environment between two markers, so that anything the startup files
/// print is ignored. The marker variable itself is left out of the listing, or its
/// value would end it early. Valid in sh, bash, zsh and fish.
const SCRIPT: &str = r#"printf '%s' "$X8AI_ENV_MARKER"; /usr/bin/env -0 -u X8AI_ENV_MARKER; printf '%s' "$X8AI_ENV_MARKER""#;

/// Variables that describe the resolving shell itself, not the user's setup.
const SHELL_STATE: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_", MARKER_VAR];

/// Variables that describe a terminal the app itself may have been started from
/// (under `pnpm tauri dev`, for example). The resolving shell must not act on
/// them: with `TERM_PROGRAM=Apple_Terminal`, macOS's `/etc/zshrc` saves and
/// restores that Terminal window's session. Agents get their own terminal's
/// values from the PTY instead.
const HOST_TERMINAL: &[&str] = &[
    "TERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "TERM_SESSION_ID",
    "COLORTERM",
    "ITERM_SESSION_ID",
    "LC_TERMINAL",
    "LC_TERMINAL_VERSION",
    "TMUX",
    "TMUX_PANE",
];

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not start {shell}: {detail}")]
    Start { shell: String, detail: String },
    #[error("{shell} did not finish starting within {} s", .timeout.as_secs())]
    Timeout { shell: String, timeout: Duration },
    #[error("{shell} did not print an environment (it exited with {status})")]
    NoOutput { shell: String, status: String },
}

/// Runs `shell` as an interactive login shell in `home` and returns the
/// environment it ends up with, without the variables that describe the shell
/// itself. The shell inherits this process's environment, as a terminal does.
pub fn resolve(
    shell: &Path,
    home: &Path,
    timeout: Duration,
) -> Result<Vec<(String, String)>, Error> {
    let shown = shell.display().to_string();
    let marker = marker();
    let mut command = Command::new(shell);
    for name in HOST_TERMINAL {
        command.env_remove(name);
    }
    let mut child = command
        .args(["-l", "-i", "-c", SCRIPT])
        .current_dir(home)
        .env("HOME", home)
        // No terminal: startup files should not try to set one up.
        .env("TERM", "dumb")
        .env(MARKER_VAR, &marker)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // Its own process group, so a timeout also ends whatever it started.
        .process_group(0)
        .spawn()
        .map_err(|e| Error::Start {
            shell: shown.clone(),
            detail: e.to_string(),
        })?;

    let mut stdout = child.stdout.take().expect("stdout is piped");
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = stdout
            .by_ref()
            .take(MAX_OUTPUT_BYTES as u64)
            .read_to_end(&mut output);
        output
    });

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                if let Ok(pid) = i32::try_from(child.id()) {
                    let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
                }
                let _ = child.wait();
                return Err(Error::Timeout {
                    shell: shown,
                    timeout,
                });
            }
        }
    };
    // Something the shell started in the background may still hold stdout open.
    if let Ok(pid) = i32::try_from(child.id()) {
        let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
    }
    let output = reader.join().unwrap_or_default();
    parse(&output, &marker).ok_or_else(|| Error::NoOutput {
        shell: shown,
        status: status.to_string(),
    })
}

/// The `env -0` output between the first two markers.
fn parse(output: &[u8], marker: &str) -> Option<Vec<(String, String)>> {
    let marker = marker.as_bytes();
    let start = find(output, marker)? + marker.len();
    let end = start + find(&output[start..], marker)?;
    let vars = output[start..end]
        .split(|&b| b == 0)
        .filter_map(|entry| {
            let entry = std::str::from_utf8(entry).ok()?;
            let (name, value) = entry.split_once('=')?;
            let valid = !name.is_empty()
                && !name.contains(['\n', '\0'])
                && !SHELL_STATE.contains(&name)
                && !HOST_TERMINAL.contains(&name);
            valid.then(|| (name.to_owned(), value.to_owned()))
        })
        .collect();
    Some(vars)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Different on every call, so startup files cannot print it by accident.
fn marker() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("__x8ai_env_{}_{nanos}__", std::process::id())
}

/// The value of `name` in an environment.
pub fn var<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
    env.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_what_is_between_the_markers() {
        let output = b"Welcome to your shell!\nM1PATH=/usr/bin:/bin\0HOME=/Users/me\0PWD=/Users/me\0MULTI=a\nb\0M1 trailing noise";
        let vars = parse(output, "M1").unwrap();
        assert_eq!(
            vars,
            vec![
                ("PATH".into(), "/usr/bin:/bin".into()),
                ("HOME".into(), "/Users/me".into()),
                ("MULTI".into(), "a\nb".into()),
            ]
        );
    }

    #[test]
    fn the_resolving_script_leaves_the_marker_out_of_the_listing() {
        // Its value is the marker, which would otherwise end the listing early.
        assert!(SCRIPT.contains("env -0 -u X8AI_ENV_MARKER"));
        assert_eq!(MARKER_VAR, "X8AI_ENV_MARKER");
    }

    #[test]
    fn output_without_markers_is_not_an_environment() {
        assert!(parse(b"PATH=/usr/bin\0", "M1").is_none());
        assert!(parse(b"M1PATH=/usr/bin\0", "M1").is_none());
    }
}
