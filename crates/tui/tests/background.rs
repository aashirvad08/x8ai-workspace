//! `x8ai` end to end, step 5: the background `x8ai`. Closing the terminal
//! leaves a space's shell and its agent running, and the next `x8ai` comes
//! back to them as they were; one terminal at a time has it; Ctrl-g d and
//! `/detach` let go of it; `x8ai --stop` ends it; with nothing open, it ends
//! by itself; and a terminal of another protocol is told what to do.

mod common;

use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::Command;

use common::{CTRL_G, X8ai, eventually, server_pid};

fn script(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(["-c", "user.name=Test", "-c", "user.email=test@example.com"])
        .args(args)
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// The highest `TICK <n>` on the screen.
fn last_tick(screen: &str) -> u32 {
    screen
        .split("TICK ")
        .skip(1)
        .filter_map(|rest| {
            rest.split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse()
                .ok()
        })
        .max()
        .unwrap_or(0)
}

fn env(bin: &Path) -> Vec<(&'static str, String)> {
    vec![
        (
            "PATH",
            format!("{}:/usr/bin:/bin:/usr/sbin:/sbin", bin.display()),
        ),
        ("GIT_CONFIG_GLOBAL", "/dev/null".to_owned()),
    ]
}

/// `x8ai <args>` run without a terminal, as from a script.
fn x8ai_command(home: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_x8ai"))
        .args(args)
        .env("HOME", home)
        .env("X8AI_DATA_DIR", home.join(".x8ai-data"))
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    (output.status.success(), text)
}

#[test]
fn a_shell_and_an_agent_outlive_the_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let proj = home.join("proj");
    fs::create_dir(&proj).unwrap();
    fs::write(proj.join("README.md"), "hello\n").unwrap();
    git(&proj, &["init", "-q"]);
    git(&proj, &["add", "."]);
    git(&proj, &["commit", "-qm", "first"]);
    let bin = home.join("bin");
    fs::create_dir(&bin).unwrap();
    // An agent that keeps working: a line every tenth of a second.
    script(
        &bin.join("claude"),
        "#!/bin/sh\ni=0\nwhile true; do i=$((i+1)); echo \"TICK $i\"; sleep 0.1; done\n",
    );
    let mut first = X8ai::start_with(&home, &["proj"], &env(&bin));
    first.wait_for("~/proj · not trusted");
    // A shell variable: only this shell has it.
    first.keys("MARK=ONE; printf 'SET-%s\\n' \"$MARK\"\r");
    first.wait_for("SET-ONE");
    // x8ai in one of its own panes would attach to itself: it says so.
    first.keys(&format!("{}\r", env!("CARGO_BIN_EXE_x8ai")));
    first.wait_for("this terminal is inside x8ai already");

    first.keys(&format!("{CTRL_G}a"));
    first.wait_for("Claude Code  installed");
    first.keys("\r");
    first.wait_until("trust asked", |s| s.contains("y trust"));
    first.keys("y");
    first.wait_until("the approval asked", |s| s.contains("y allow"));
    first.keys("y");
    let screen = first.wait_until("the agent working", |s| last_tick(s) >= 3);
    let before = last_tick(&screen);

    // The terminal closes; everything keeps running.
    first.hang_up();
    let pid = server_pid(&home).expect("the background x8ai runs on");
    std::thread::sleep(std::time::Duration::from_millis(800));
    assert_eq!(server_pid(&home), Some(pid));

    // The next x8ai comes back to it as it was left: the agent went on.
    let mut second = X8ai::start_with(&home, &[], &env(&bin));
    let screen = second.wait_until("the agent seen again, further on", |s| {
        last_tick(s) >= before + 5
    });
    assert!(screen.contains("~/proj · trusted"), "{screen}");
    // The shell too: the same one, with its variable.
    second.keys(&format!("{CTRL_G}1"));
    second.wait_until("the shell shown", |s| !s.contains("TICK"));
    second.keys("printf 'STILL-%s\\n' \"$MARK\"\r");
    second.wait_for("STILL-ONE");
    assert_eq!(server_pid(&home), Some(pid));

    // Quitting ends it all, before the terminal is given back.
    second.keys(&format!("{CTRL_G}q"));
    let screen = second.wait_until("quit asked", |s| s.contains("y quit"));
    assert!(screen.contains("Ctrl-g d"), "{screen}");
    second.keys("y");
    assert_eq!(second.wait_for_exit().code, 0);
    assert_eq!(server_pid(&home), None);
    assert!(!home.join(".x8ai/server/x8ai.sock").exists());
}

#[test]
fn one_terminal_at_a_time_detached_and_stopped() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    fs::create_dir(home.join("proj")).unwrap();
    fs::create_dir(home.join("other")).unwrap();
    let bin = home.join("bin");
    fs::create_dir(&bin).unwrap();
    let mut first = X8ai::start_with(&home, &["proj"], &env(&bin));
    first.wait_for("~/proj · not trusted");

    // A terminal of another protocol is told how to go on, and nothing else
    // changes.
    let mut stream = UnixStream::connect(home.join(".x8ai/server/x8ai.sock")).unwrap();
    stream.set_read_timeout(Some(common::TIMEOUT)).unwrap();
    let hello = br#"{"Hello":{"protocol":999,"version":"9.9.9","cwd":"/","folder":null,"size":[80,24],"truecolor":false}}"#;
    let mut frame = vec![1u8];
    frame.extend_from_slice(&(hello.len() as u32).to_be_bytes());
    frame.extend_from_slice(hello);
    stream.write_all(&frame).unwrap();
    let mut reply = Vec::new();
    stream.read_to_end(&mut reply).unwrap();
    let reply = String::from_utf8_lossy(&reply[5..]).into_owned();
    assert!(reply.contains("cannot talk to x8ai 9.9.9"), "{reply}");
    assert!(reply.contains("x8ai --stop"), "{reply}");

    // Another terminal takes it over; the first one lets go.
    let mut second = X8ai::start_with(&home, &[], &env(&bin));
    second.wait_for("~/proj · not trusted");
    first.wait_for("x8ai is open in another terminal now.");
    assert_eq!(first.wait_for_exit().code, 0);

    // Ctrl-g d: the terminal lets go, x8ai keeps running.
    second.keys(&format!("{CTRL_G}d"));
    second.wait_for("x8ai keeps running in the background. Run x8ai to come back to it.");
    assert_eq!(second.wait_for_exit().code, 0);
    let pid = server_pid(&home).expect("still running");

    // x8ai <folder> comes back and opens it; /detach on the Welcome lets go.
    let mut third = X8ai::start_with(&home, &["other"], &env(&bin));
    third.wait_for("~/other · not trusted");
    third.keys(&format!("{CTRL_G}h"));
    third.wait_for("W E L C O M E");
    third.keys("/detach\r");
    third.wait_for("x8ai keeps running in the background.");
    assert_eq!(third.wait_for_exit().code, 0);
    assert_eq!(server_pid(&home), Some(pid));

    // x8ai --stop ends it, with its shells.
    let (ok, said) = x8ai_command(&home, &["--stop"]);
    assert!(ok, "{said}");
    assert!(said.contains("x8ai is stopped"), "{said}");
    assert_eq!(server_pid(&home), None);
    let (ok, said) = x8ai_command(&home, &["--stop"]);
    assert!(ok, "{said}");
    assert!(said.contains("not running"), "{said}");
}

#[test]
fn with_nothing_open_it_ends_by_itself() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();

    // Detaching with nothing open ends it: there is nothing to keep.
    let mut x8ai = X8ai::start(&home, &[]);
    x8ai.wait_for("W E L C O M E");
    x8ai.keys("/detach\r");
    x8ai.wait_for("Nothing is open, so x8ai ends.");
    assert_eq!(x8ai.wait_for_exit().code, 0);
    assert_eq!(server_pid(&home), None);

    // Its terminal closed with nothing open: it ends soon after.
    let mut x8ai = X8ai::start(&home, &[]);
    x8ai.wait_for("W E L C O M E");
    assert!(server_pid(&home).is_some());
    x8ai.hang_up();
    eventually("the background x8ai ended", || server_pid(&home).is_none());
}
