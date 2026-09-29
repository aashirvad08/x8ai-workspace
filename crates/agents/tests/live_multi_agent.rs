//! Two real Claude Code sessions at once in a disposable Git repository, each in a
//! worktree of its own. Not run by default:
//!
//! ```sh
//! cargo test -p x8ai-agents --test live_multi_agent -- --ignored --nocapture
//! ```
//!
//! Everything is made in a temporary directory: the repository, the worktree root,
//! the trust and approval stores. Claude is never sent a prompt; the "agent's"
//! file change is written into its worktree from outside while it runs. Both
//! sessions are stopped and their worktrees removed at the end.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use x8ai_agents::environment::{RESOLVE_TIMEOUT, resolve};
use x8ai_agents::{AgentRuntime, Isolation, SessionState, authorize, builtin, plan};
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_git::Git;
use x8ai_pty::{SessionEvents, Sessions, user_shell};
use x8ai_workspace::{ApprovalStore, TrustStore};

const SIZE: TerminalSize = TerminalSize {
    cols: 120,
    rows: 36,
};

#[derive(Default)]
struct Recorder {
    state: Mutex<(usize, Option<TerminalExit>)>,
    changed: Condvar,
}

impl SessionEvents for Recorder {
    fn output(&self, bytes: Vec<u8>) {
        self.state.lock().unwrap().0 += bytes.len();
        self.changed.notify_all();
    }
    fn error(&self, _: String) {}
    fn exited(&self, exit: TerminalExit) {
        self.state.lock().unwrap().1 = Some(exit);
        self.changed.notify_all();
    }
}

impl Recorder {
    fn wait_for_output(&self, bytes: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut state = self.state.lock().unwrap();
        while state.0 < bytes {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        true
    }
}

fn run(dir: &Path, program: &str, args: &[&str]) -> String {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn cwd_of(pid: u32) -> PathBuf {
    let text = run(
        Path::new("/"),
        "lsof",
        &["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"],
    );
    PathBuf::from(
        text.lines()
            .find_map(|l| l.strip_prefix('n'))
            .unwrap_or_default(),
    )
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

#[test]
#[ignore = "needs Claude Code installed; run by hand"]
fn two_claude_code_sessions_work_in_separate_worktrees() {
    let temp = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(temp.path()).unwrap();
    let repo_dir = base.join("disposable-project");
    std::fs::create_dir_all(repo_dir.join("src")).unwrap();
    std::fs::write(repo_dir.join("README.md"), "# disposable test project\n").unwrap();
    std::fs::write(repo_dir.join("src/hello.txt"), "hello\n").unwrap();
    let git_cli = |args: &[&str]| {
        let mut all = vec![
            "-c",
            "user.name=Live Test",
            "-c",
            "user.email=live@example.com",
            "-c",
            "init.defaultBranch=main",
        ];
        all.extend_from_slice(args);
        run(&repo_dir, "git", &all)
    };
    git_cli(&["init", "-q"]);
    git_cli(&["add", "."]);
    git_cli(&["commit", "-qm", "disposable project"]);
    let primary_head = git_cli(&["rev-parse", "HEAD"]);
    println!(
        "disposable repository: {} at {}",
        repo_dir.display(),
        &primary_head[..10]
    );

    let home = std::env::home_dir().unwrap();
    let environment = resolve(&PathBuf::from(user_shell()), &home, RESOLVE_TIMEOUT).unwrap();
    let claude = builtin()
        .into_iter()
        .find(|a| a.id.as_str() == "claude-code")
        .unwrap();
    let plan = plan(&claude, &environment, &repo_dir).expect("Claude Code is installed");
    let git = Git::new(PathBuf::from("/usr/bin/git"), &environment);
    let repo = git.repository(&repo_dir).unwrap().unwrap();
    let isolation = Isolation::new(base.join("home/.x8ai/worktrees"));

    // Trust and approval first; no worktree before that.
    let (mut trust, _) = TrustStore::load(base.join("data/trusted.json"));
    let (mut approvals, _) = ApprovalStore::load(base.join("data/approvals.json"));
    assert!(authorize(&plan, &trust, &approvals).is_err());
    trust.set(&repo_dir, true).unwrap();
    assert!(authorize(&plan, &trust, &approvals).is_err());
    approvals.approve(&plan.approval()).unwrap();
    println!("trust and approval: required, then granted");

    let sessions = Sessions::default();
    let runtime = AgentRuntime::default();
    let mut started = Vec::new();
    for label in ["first", "second"] {
        let worktree = isolation
            .create(&git, &repo, &claude.id, None, &[])
            .unwrap();
        let id = runtime
            .create(&plan, worktree.path.clone(), Some(worktree.clone()))
            .unwrap();
        let recorder = Arc::new(Recorder::default());
        let authorized = authorize(&plan, &trust, &approvals).unwrap();
        let pty = runtime
            .run(&sessions, id, authorized, SIZE, recorder.clone())
            .unwrap();
        let pid = pty.pid().unwrap();
        let drew = recorder.wait_for_output(200);
        let cwd = cwd_of(pid);
        println!("{label} Claude Code: pid {pid}, drew its interface: {drew}");
        println!(
            "  worktree {} on {}",
            worktree.path.display(),
            worktree.branch
        );
        println!("  cwd      {}", cwd.display());
        assert_eq!(cwd, worktree.path);
        started.push((id, worktree, pty, pid, recorder));
    }
    let (one, two) = (&started[0], &started[1]);
    assert_ne!(one.1.path, two.1.path);
    assert_ne!(one.1.branch, two.1.branch);
    assert!(
        runtime
            .sessions_in(&repo_dir)
            .iter()
            .all(|s| s.state == SessionState::Running)
    );
    println!("two sessions running at once, in different directories");

    // A harmless change in the first agent's worktree, while it runs.
    std::fs::write(
        one.1.path.join("AGENT_NOTE.md"),
        "A harmless note in the agent's worktree.\n",
    )
    .unwrap();
    std::fs::write(
        one.1.path.join("src/hello.txt"),
        "hello from the agent's worktree\n",
    )
    .unwrap();

    // The user's working tree is untouched.
    assert_eq!(git_cli(&["status", "--porcelain"]), "");
    assert_eq!(git_cli(&["rev-parse", "HEAD"]), primary_head);
    assert_eq!(
        std::fs::read_to_string(repo_dir.join("src/hello.txt")).unwrap(),
        "hello\n"
    );
    assert!(!repo_dir.join("AGENT_NOTE.md").exists());
    println!("primary working tree: unchanged (clean, same commit)");

    // Each session's changes, as the app shows them.
    for (label, (_, worktree, ..)) in [("first", one), ("second", two)] {
        let changes = git.changes(&worktree.path, &worktree.base).unwrap();
        let files: Vec<String> = changes
            .files
            .iter()
            .map(|f| format!("{:?} {}", f.status, f.path))
            .collect();
        println!("{label} session changes: {files:?}");
    }
    let changes = git.changes(&one.1.path, &one.1.base).unwrap();
    assert_eq!(changes.files.len(), 2);
    assert!(changes.diff.contains("+hello from the agent's worktree"));
    assert!(
        git.changes(&two.1.path, &two.1.base)
            .unwrap()
            .files
            .is_empty()
    );

    // Stopping one leaves the other running.
    runtime.stop(&sessions, one.0).unwrap();
    assert!(gone_within(one.3, Duration::from_secs(5)));
    assert!(alive(two.3));
    println!("stopped the first; the second still runs");
    runtime.stop(&sessions, two.0).unwrap();
    assert!(gone_within(two.3, Duration::from_secs(5)));
    assert!(!runtime.any_running());
    let leftovers = run(
        Path::new("/"),
        "pgrep",
        &["-f", &base.display().to_string()],
    );
    assert!(leftovers.is_empty(), "left running: {leftovers}");
    println!("both stopped; no processes left");

    // Clean up: the first has uncommitted work, so it takes an explicit discard.
    assert!(isolation.remove(&git, &repo, &one.1, false).is_err());
    for (_, worktree, ..) in &started {
        let removal = isolation.remove(&git, &repo, worktree, true).unwrap();
        assert_eq!(removal.kept_branch, None);
        assert!(!worktree.path.exists());
    }
    assert!(git.worktrees(&repo).unwrap().len() == 1);
    assert_eq!(git_cli(&["branch", "--list", "agent/*"]), "");
    println!("worktrees and branches removed; the repository is as it started");
}
