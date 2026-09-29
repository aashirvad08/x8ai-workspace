//! Resolving the user's login-shell environment with real shells.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use nix::sys::signal::kill;
use nix::unistd::Pid;
use x8ai_agents::environment::{Error, resolve, var};

fn home() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let home = fs::canonicalize(temp.path()).unwrap();
    (temp, home)
}

#[test]
fn reads_what_the_login_shell_sets_up_and_ignores_what_it_prints() {
    let (_t, home) = home();
    fs::write(
        home.join(".profile"),
        "echo 'Welcome! Here is some noise.'\n\
         export AGENT_ENV_TEST='from profile'\n\
         export PATH=\"$HOME/tools:$PATH\"\n\
         pwd > \"$HOME/ran-in\"\n",
    )
    .unwrap();

    let env = resolve(Path::new("/bin/sh"), &home, Duration::from_secs(10)).unwrap();
    assert_eq!(var(&env, "AGENT_ENV_TEST"), Some("from profile"));
    assert!(
        var(&env, "PATH")
            .unwrap()
            .starts_with(&format!("{}/tools:", home.display()))
    );
    assert_eq!(var(&env, "HOME"), Some(home.to_str().unwrap()));
    // The resolving shell's own state is not part of the result.
    for name in ["PWD", "OLDPWD", "SHLVL", "_", "X8AI_ENV_MARKER"] {
        assert!(var(&env, name).is_none(), "{name} leaked");
    }
    // It ran in the home directory, never in a workspace.
    let ran_in = fs::read_to_string(home.join("ran-in")).unwrap();
    assert_eq!(fs::canonicalize(ran_in.trim()).unwrap(), home);
}

#[test]
fn reads_the_interactive_startup_file_where_tools_usually_add_themselves() {
    // `~/.local/bin` (Claude Code's installer) is typically added in `.zshrc`,
    // which only an interactive shell reads.
    let zsh = Path::new("/bin/zsh");
    if !zsh.exists() {
        eprintln!("skipped: no /bin/zsh");
        return;
    }
    let (_t, home) = home();
    fs::write(home.join(".zshrc"), "export FROM_ZSHRC=interactive\n").unwrap();
    // Twice: on macOS, a first run under Terminal.app would have saved a session
    // that the second restores, if the host terminal's variables reached zsh.
    for _ in 0..2 {
        let env = resolve(zsh, &home, Duration::from_secs(10)).unwrap();
        assert_eq!(var(&env, "FROM_ZSHRC"), Some("interactive"));
        assert!(var(&env, "TERM_SESSION_ID").is_none());
    }
    assert!(
        !home.join(".zsh_sessions").exists(),
        "a terminal session was saved"
    );
}

// The next two use the real /bin/sh with startup files that misbehave, rather than
// freshly written scripts: macOS scans a new executable on its first run, which
// can take longer than the timeout under test.

#[test]
fn a_startup_file_that_hangs_is_abandoned_and_what_it_started_is_killed() {
    let (_t, home) = home();
    let pid_file = home.join("pid");
    fs::write(
        home.join(".profile"),
        format!("sleep 30 &\necho $! > '{}'\nwait\n", pid_file.display()),
    )
    .unwrap();

    let started = Instant::now();
    let result = resolve(Path::new("/bin/sh"), &home, Duration::from_millis(1500));
    assert!(matches!(result, Err(Error::Timeout { .. })), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(5));

    let child: i32 = fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while kill(Pid::from_raw(child), None).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the startup file's child survived"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_shell_that_prints_no_environment_is_an_error() {
    let (_t, home) = home();
    fs::write(home.join(".profile"), "echo 'bye'\nexit 0\n").unwrap();
    assert!(matches!(
        resolve(Path::new("/bin/sh"), &home, Duration::from_secs(5)),
        Err(Error::NoOutput { .. })
    ));
    assert!(matches!(
        resolve(&home.join("missing-shell"), &home, Duration::from_secs(5)),
        Err(Error::Start { .. })
    ));
}
