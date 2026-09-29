//! The runtime with the real Claude Code, on a machine where it is installed.
//! Not run by default:
//!
//! ```sh
//! cargo test -p x8ai-agents --test live_claude -- --ignored --nocapture
//! ```
//!
//! It reads the user's real login environment, finds `claude` on its `PATH`,
//! checks trust and approval with throwaway stores, starts Claude Code on a PTY in
//! a scratch workspace (`X8AI_LIVE_WORKSPACE`, or a temporary folder), and checks
//! its directory, environment and terminal. It never sends Claude a prompt: it
//! only presses Ctrl+C, and closes and shuts down sessions. Claude Code may record
//! the scratch folder in its own settings, as when run by hand.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use x8ai_agents::environment::{RESOLVE_TIMEOUT, resolve, var};
use x8ai_agents::{AgentRuntime, Denied, authorize, builtin, plan};
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_pty::{SessionEvents, Sessions, user_shell};
use x8ai_workspace::{ApprovalStore, TrustStore};

const SIZE: TerminalSize = TerminalSize {
    cols: 120,
    rows: 36,
};

#[derive(Default)]
struct Recorder {
    state: Mutex<(Vec<u8>, Option<TerminalExit>)>,
    changed: Condvar,
}

impl SessionEvents for Recorder {
    fn output(&self, bytes: Vec<u8>) {
        self.state.lock().unwrap().0.extend(bytes);
        self.changed.notify_all();
    }
    fn error(&self, message: String) {
        eprintln!("session error: {message}");
    }
    fn exited(&self, exit: TerminalExit) {
        self.state.lock().unwrap().1 = Some(exit);
        self.changed.notify_all();
    }
}

impl Recorder {
    fn bytes(&self) -> usize {
        self.state.lock().unwrap().0.len()
    }
    /// The output as text, without terminal escape sequences.
    fn text(&self) -> String {
        let raw = String::from_utf8_lossy(&self.state.lock().unwrap().0).into_owned();
        let mut text = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                // Skip CSI/OSC sequences.
                match chars.next() {
                    Some('[') => {
                        let mut params = String::new();
                        for c in chars.by_ref() {
                            if ('@'..='~').contains(&c) {
                                // Cursor forward stands in for spaces in TUIs.
                                if c == 'C' {
                                    let n = params.parse().unwrap_or(1).min(200);
                                    text.extend(std::iter::repeat_n(' ', n));
                                }
                                break;
                            }
                            params.push(c);
                        }
                    }
                    Some(']') => {
                        while let Some(c) = chars.next() {
                            if c == '\u{7}' || (c == '\u{1b}' && chars.peek() == Some(&'\\')) {
                                break;
                            }
                        }
                    }
                    _ => {}
                }
            } else if !c.is_control() || c == '\n' {
                text.push(c);
            }
        }
        text
    }
    fn wait_for_output(&self, min_bytes: usize, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        while state.0.len() < min_bytes {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        true
    }
    fn wait_for_exit(&self, timeout: Duration) -> Option<TerminalExit> {
        let deadline = Instant::now() + timeout;
        let mut state = self.state.lock().unwrap();
        while state.1.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        state.1.clone()
    }
}

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

fn gone_within(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while alive(pid) {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

/// The process and everything below it.
fn descendants(pid: u32) -> Vec<u32> {
    let output = Command::new("ps")
        .args(["-A", "-o", "pid=,ppid="])
        .output()
        .unwrap();
    let pairs: Vec<(u32, u32)> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            Some((f.next()?.parse().ok()?, f.next()?.parse().ok()?))
        })
        .collect();
    let mut found = vec![pid];
    let mut i = 0;
    while i < found.len() {
        let parent = found[i];
        found.extend(pairs.iter().filter(|(_, p)| *p == parent).map(|(c, _)| *c));
        i += 1;
    }
    found
}

fn cwd_of(pid: u32) -> String {
    let output = Command::new("lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|l| l.strip_prefix('n'))
        .unwrap_or_default()
        .to_owned()
}

/// The process's environment, as `ps -E` shows it (the user's own processes only).
fn environment_of(pid: u32, names: &[&str]) -> Vec<String> {
    let output = Command::new("ps")
        .args(["-Eww", "-o", "command=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.split(' ')
        .filter(|word| names.iter().any(|n| word.starts_with(&format!("{n}="))))
        .map(str::to_owned)
        .collect()
}

#[test]
#[ignore = "needs Claude Code installed; run by hand"]
fn claude_code_runs_through_the_runtime() {
    let home = std::env::home_dir().unwrap();
    let shell = PathBuf::from(user_shell());
    let started = Instant::now();
    let environment = resolve(&shell, &home, RESOLVE_TIMEOUT).expect("login environment");
    println!(
        "login environment from {} in {:?}: {} variables",
        shell.display(),
        started.elapsed(),
        environment.len()
    );

    let temp = tempfile::tempdir().unwrap();
    let workspace = match std::env::var_os("X8AI_LIVE_WORKSPACE") {
        Some(dir) => std::fs::canonicalize(dir).unwrap(),
        None => {
            let dir = temp.path().join("live-workspace");
            std::fs::create_dir(&dir).unwrap();
            std::fs::canonicalize(dir).unwrap()
        }
    };
    let claude = builtin()
        .into_iter()
        .find(|a| a.id.as_str() == "claude-code")
        .unwrap();
    let plan = plan(&claude, &environment, &workspace).expect("Claude Code is installed");
    println!(
        "resolved: {} {:?} in {}",
        plan.program.display(),
        plan.args,
        plan.workspace.display()
    );
    assert!(plan.program.is_absolute());

    // The gate, with throwaway stores.
    let (mut trust, _) = TrustStore::load(temp.path().join("data/trusted.json"));
    let (mut approvals, _) = ApprovalStore::load(temp.path().join("data/approvals.json"));
    assert!(matches!(
        authorize(&plan, &trust, &approvals),
        Err(Denied::Untrusted(_))
    ));
    println!("untrusted workspace: blocked");
    trust.set(&workspace, true).unwrap();
    assert!(matches!(
        authorize(&plan, &trust, &approvals),
        Err(Denied::NotApproved { .. })
    ));
    println!("trusted, not approved: blocked");
    approvals.approve(&plan.approval()).unwrap();

    let sessions = Sessions::default();
    let runtime = AgentRuntime::default();
    let start = |label: &str| {
        let recorder = Arc::new(Recorder::default());
        let authorized = authorize(&plan, &trust, &approvals).expect("authorized");
        let session = runtime
            .start(&sessions, authorized, SIZE, recorder.clone())
            .expect("started");
        let pid = session.pid().unwrap();
        let drew = recorder.wait_for_output(200, Duration::from_secs(20));
        println!(
            "{label}: pid {pid}, drew its interface: {drew} ({} bytes)",
            recorder.bytes()
        );
        (session, recorder, pid)
    };

    // 1. It runs in the workspace, with the login environment, on a terminal.
    let (session, recorder, pid) = start("start 1");
    let screen = recorder.text();
    let excerpt: String = screen
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(12)
        .collect::<Vec<_>>()
        .join(" | ");
    println!("screen: {excerpt}");
    let cwd = cwd_of(pid);
    println!("cwd: {cwd}");
    assert_eq!(Path::new(&cwd), workspace);
    let env = environment_of(
        pid,
        &[
            "TERM",
            "TERM_PROGRAM",
            "TERM_SESSION_ID",
            "COLORTERM",
            "PATH",
        ],
    );
    for line in &env {
        let shown = if line.starts_with("PATH=") && line.len() > 90 {
            format!("{}…", &line[..90])
        } else {
            line.clone()
        };
        println!("env: {shown}");
    }
    assert!(env.contains(&"TERM=xterm-256color".to_owned()));
    assert!(env.contains(&"TERM_PROGRAM=x8ai-workspace".to_owned()));
    assert!(!env.iter().any(|v| v.starts_with("TERM_SESSION_ID=")));
    assert_eq!(
        var(&plan.env, "PATH"),
        env.iter().find_map(|v| v.strip_prefix("PATH="))
    );

    // 2. Ctrl+C reaches it: Claude Code exits on the second press.
    session.write(vec![0x03]).unwrap();
    std::thread::sleep(Duration::from_millis(600));
    if session.wait_for_exit(Duration::from_millis(100)) {
        println!("ctrl+c: exited after one press");
    } else {
        session.write(vec![0x03]).unwrap();
    }
    let exit = recorder
        .wait_for_exit(Duration::from_secs(10))
        .expect("Claude Code exited after Ctrl+C");
    println!("ctrl+c: exit {exit:?}");
    assert!(gone_within(pid, Duration::from_secs(5)));

    // 3. Closing its terminal ends it and everything it started.
    let (session, _recorder, pid) = start("start 2");
    std::thread::sleep(Duration::from_secs(2));
    let tree = descendants(pid);
    println!("process tree before close: {tree:?}");
    sessions.close(session.id()).unwrap();
    for p in &tree {
        assert!(
            gone_within(*p, Duration::from_secs(5)),
            "{p} survived closing the terminal"
        );
    }
    println!("close: all {} processes gone", tree.len());

    // 4. Quitting the app ends it.
    let (_session, _recorder, pid) = start("start 3");
    std::thread::sleep(Duration::from_secs(2));
    let tree = descendants(pid);
    sessions.shutdown(Duration::from_millis(500));
    for p in &tree {
        assert!(
            gone_within(*p, Duration::from_secs(5)),
            "{p} survived app shutdown"
        );
    }
    println!("quit: all {} processes gone", tree.len());
    assert!(!runtime.any_running());
}
