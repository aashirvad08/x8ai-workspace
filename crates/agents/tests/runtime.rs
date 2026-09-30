//! The agent runtime against real processes on real PTYs. A small shell script
//! stands in for an agent: it reports its directory, environment and terminal,
//! echoes its input, and exits when told to.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use x8ai_agents::{AgentRuntime, Denied, LaunchPlan, RunError, SessionState, authorize, plan};
use x8ai_core::agent::AgentDefinition;
use x8ai_core::agent::AgentSessionId;
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_pty::{SessionEvents, Sessions};
use x8ai_workspace::{ApprovalStore, TrustStore};

const SIZE: TerminalSize = TerminalSize {
    cols: 100,
    rows: 30,
};
const TIMEOUT: Duration = Duration::from_secs(10);

const FAKE_AGENT: &str = r#"#!/bin/sh
trap 'echo "interrupted"; exit 130' INT
echo "cwd=$(pwd)"
echo "marker=$AGENT_TEST_MARKER"
echo "app-only=${CARGO_MANIFEST_DIR:-absent}"
echo "term=$TERM"
if [ -t 0 ] && [ -t 1 ]; then echo "tty=yes"; else echo "tty=no"; fi
echo "size=$(stty size)"
echo "ready"
while read -r line; do
  case "$line" in
    quit) exit 3 ;;
    wait) sleep 30 & wait ;;
    *) echo "got:$line" ;;
  esac
done
"#;

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
    fn error(&self, _: String) {}
    fn exited(&self, exit: TerminalExit) {
        self.state.lock().unwrap().1 = Some(exit);
        self.changed.notify_all();
    }
}

impl Recorder {
    fn output(&self) -> String {
        String::from_utf8_lossy(&self.state.lock().unwrap().0).into_owned()
    }
    fn wait_for(&self, needle: &str) {
        let deadline = Instant::now() + TIMEOUT;
        let mut state = self.state.lock().unwrap();
        while !String::from_utf8_lossy(&state.0).contains(needle) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "no {needle:?} in {:?}",
                String::from_utf8_lossy(&state.0)
            );
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
    }
    fn wait_for_exit(&self) -> TerminalExit {
        let deadline = Instant::now() + TIMEOUT;
        let mut state = self.state.lock().unwrap();
        while state.1.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "the agent did not exit");
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        state.1.clone().unwrap()
    }
}

/// A workspace, a directory with the fake agent on "PATH", and the app's stores.
struct Fixture {
    _temp: tempfile::TempDir,
    workspace: PathBuf,
    other: PathBuf,
    bin: PathBuf,
    trust: TrustStore,
    approvals: ApprovalStore,
    sessions: Sessions,
    runtime: AgentRuntime,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let workspace = root.join("project");
    let other = root.join("other-project");
    let bin = root.join("bin");
    for dir in [&workspace, &other, &bin] {
        fs::create_dir(dir).unwrap();
    }
    let agent = bin.join("fake-agent");
    fs::write(&agent, FAKE_AGENT).unwrap();
    fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).unwrap();
    let (trust, _) = TrustStore::load(root.join("data/trusted.json"));
    let (approvals, _) = ApprovalStore::load(root.join("data/approvals.json"));
    Fixture {
        _temp: temp,
        workspace,
        other,
        bin,
        trust,
        approvals,
        sessions: Sessions::default(),
        runtime: AgentRuntime::default(),
    }
}

fn definition(program: &str) -> AgentDefinition {
    serde_json::from_value(serde_json::json!({
        "id": "fake-agent",
        "name": "Fake Agent",
        "launch": { "program": program },
        "capabilities": {}
    }))
    .unwrap()
}

impl Fixture {
    /// The login environment the runtime would have resolved.
    fn environment(&self) -> Vec<(String, String)> {
        vec![
            (
                "PATH".into(),
                format!("{}:/usr/bin:/bin", self.bin.display()),
            ),
            (
                "HOME".into(),
                self.workspace.parent().unwrap().display().to_string(),
            ),
            ("AGENT_TEST_MARKER".into(), "from-login-shell".into()),
        ]
    }

    fn plan(&self, workspace: &Path) -> LaunchPlan {
        plan(&definition("fake-agent"), &self.environment(), workspace).unwrap()
    }

    fn trust_and_approve(&mut self, plan: &LaunchPlan) {
        self.trust.set(&plan.workspace, true).unwrap();
        self.approvals.approve(&plan.approval()).unwrap();
    }

    /// A session directly in the plan's workspace (these are not Git
    /// repositories), and the agent running in it.
    fn start(&self, plan: &LaunchPlan) -> (Arc<x8ai_pty::Session>, Arc<Recorder>) {
        let (session, recorder, _) = self.start_session(plan);
        (session, recorder)
    }

    fn start_session(
        &self,
        plan: &LaunchPlan,
    ) -> (Arc<x8ai_pty::Session>, Arc<Recorder>, AgentSessionId) {
        let id = self
            .runtime
            .create(plan, plan.workspace.clone(), None)
            .unwrap();
        let (session, recorder) = self.run(id, plan);
        (session, recorder, id)
    }

    fn run(
        &self,
        id: AgentSessionId,
        plan: &LaunchPlan,
    ) -> (Arc<x8ai_pty::Session>, Arc<Recorder>) {
        let recorder = Arc::new(Recorder::default());
        let authorized = authorize(plan, &self.trust, &self.approvals).expect("authorized");
        let session = self
            .runtime
            .run(&self.sessions, id, authorized, SIZE, recorder.clone())
            .unwrap();
        recorder.wait_for("ready");
        (session, recorder)
    }
}

fn alive(pid: u32) -> bool {
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

fn wait_until_gone(pid: u32) -> bool {
    let deadline = Instant::now() + TIMEOUT;
    while alive(pid) {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

#[test]
fn discovery_reports_installed_and_missing_agents() {
    let f = fixture();
    let found = f.plan(&f.workspace);
    assert_eq!(found.program, f.bin.join("fake-agent"));

    let missing = plan(&definition("no-such-agent"), &f.environment(), &f.workspace);
    assert!(matches!(missing, Err(Denied::NotInstalled { .. })));
    // Found only with the user's PATH, not the app's.
    let without_path = plan(&definition("fake-agent"), &[], &f.workspace);
    assert!(matches!(without_path, Err(Denied::NotInstalled { .. })));
}

#[test]
fn an_untrusted_workspace_blocks_the_launch() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    // Even an existing approval does not help while the folder is untrusted.
    f.approvals.approve(&plan.approval()).unwrap();
    let denied = authorize(&plan, &f.trust, &f.approvals).unwrap_err();
    assert_eq!(denied, Denied::Untrusted(f.workspace.clone()));
}

#[test]
fn a_trusted_workspace_without_approval_blocks_the_launch() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust.set(&f.workspace, true).unwrap();
    let denied = authorize(&plan, &f.trust, &f.approvals).unwrap_err();
    assert!(matches!(denied, Denied::NotApproved { .. }));
}

#[test]
fn an_approval_for_one_workspace_does_not_cover_another() {
    let mut f = fixture();
    let approved = f.plan(&f.workspace);
    f.trust_and_approve(&approved);
    let elsewhere = f.plan(&f.other);
    f.trust.set(&f.other, true).unwrap();
    assert!(matches!(
        authorize(&elsewhere, &f.trust, &f.approvals),
        Err(Denied::NotApproved { .. })
    ));
}

#[test]
fn a_different_executable_on_the_path_needs_a_new_approval() {
    let mut f = fixture();
    let approved = f.plan(&f.workspace);
    f.trust_and_approve(&approved);
    // Something earlier on PATH now answers to the same name.
    let earlier = f.workspace.parent().unwrap().join("earlier");
    fs::create_dir(&earlier).unwrap();
    let impostor = earlier.join("fake-agent");
    fs::write(&impostor, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&impostor, fs::Permissions::from_mode(0o755)).unwrap();
    let mut env = f.environment();
    env[0].1 = format!("{}:{}", earlier.display(), env[0].1);
    let changed = plan(&definition("fake-agent"), &env, &f.workspace).unwrap();
    assert_eq!(changed.program, impostor);
    assert!(matches!(
        authorize(&changed, &f.trust, &f.approvals),
        Err(Denied::NotApproved { .. })
    ));
}

#[test]
fn an_approved_agent_runs_in_its_workspace_with_the_login_environment_on_a_pty() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust_and_approve(&plan);
    // Cargo sets this in the test process, the app side of the launch. The agent
    // must not see it: it gets the plan's environment and nothing else.
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
    assert!(
        plan.env
            .iter()
            .all(|(name, _)| name != "CARGO_MANIFEST_DIR")
    );
    let (session, recorder) = f.start(&plan);

    let output = recorder.output();
    assert!(
        output.contains(&format!("cwd={}", f.workspace.display())),
        "{output}"
    );
    assert!(output.contains("marker=from-login-shell"), "{output}");
    assert!(output.contains("app-only=absent"), "{output}");
    assert!(output.contains("term=xterm-256color"), "{output}");
    assert!(output.contains("tty=yes"), "{output}");
    assert!(output.contains("size=30 100"), "{output}");

    let status = f.runtime.session_of(session.id()).unwrap();
    assert_eq!(status.state, SessionState::Running);
    assert_eq!(status.workspace, f.workspace);
    assert_eq!(status.cwd, f.workspace);
    assert!(status.worktree.is_none());
    assert_eq!(status.terminal, Some(session.id()));
    assert_eq!(status.agent.as_str(), "fake-agent");
}

#[test]
fn input_reaches_the_agent_and_its_exit_is_detected() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust_and_approve(&plan);
    let (session, recorder) = f.start(&plan);

    session.write(b"hello agent\n".to_vec()).unwrap();
    recorder.wait_for("got:hello agent");
    session.write(b"quit\n".to_vec()).unwrap();
    let exit = recorder.wait_for_exit();
    assert_eq!(exit.code, 3);
    assert!(matches!(
        f.runtime.sessions()[0].state,
        SessionState::Exited(_)
    ));
    assert!(!f.runtime.any_running());
}

#[test]
fn ctrl_c_reaches_the_agent() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust_and_approve(&plan);
    let (session, recorder) = f.start(&plan);

    session.write(vec![0x03]).unwrap();
    recorder.wait_for("interrupted");
    assert_eq!(recorder.wait_for_exit().code, 130);
}

#[test]
fn closing_the_agents_terminal_ends_the_agent_and_its_children() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust_and_approve(&plan);
    let (session, _recorder, id) = f.start_session(&plan);
    let pid = session.pid().unwrap();
    // The agent is busy with a child process when the terminal closes.
    session.write(b"wait\n".to_vec()).unwrap();
    std::thread::sleep(Duration::from_millis(200));

    f.runtime.stop(&f.sessions, id).unwrap();
    assert!(wait_until_gone(pid), "the agent outlived its terminal");
    assert!(f.sessions.get(session.id()).is_err());
    // The session stays, no longer running, so the agent can be restarted.
    assert!(session.wait_for_exit(TIMEOUT));
    assert_ne!(f.runtime.get(id).unwrap().state, SessionState::Running);
    assert!(!f.runtime.any_running());
}

#[test]
fn closing_the_terminal_directly_ends_the_agent_too() {
    // What `terminal_close` does: the session registry closes the session, and the
    // runtime learns of it from the session itself.
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust_and_approve(&plan);
    let (session, _recorder) = f.start(&plan);
    let pid = session.pid().unwrap();

    f.sessions.close(session.id()).unwrap();
    assert!(wait_until_gone(pid), "the agent outlived its terminal");
    assert!(session.wait_for_exit(TIMEOUT));
    assert!(matches!(
        f.runtime.sessions()[0].state,
        SessionState::Exited(_)
    ));
    assert!(!f.runtime.any_running());
}

#[test]
fn quitting_the_app_ends_every_agent() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust_and_approve(&plan);
    let elsewhere = f.plan(&f.other);
    f.trust_and_approve(&elsewhere);
    let (first, _) = f.start(&plan);
    let (second, _) = f.start(&elsewhere);
    let pids = [first.pid().unwrap(), second.pid().unwrap()];
    assert!(f.runtime.any_running());

    f.sessions.shutdown(Duration::from_millis(500));
    for pid in pids {
        assert!(wait_until_gone(pid), "agent {pid} survived app shutdown");
    }
    assert!(!f.runtime.any_running());
}

#[test]
fn an_agent_killed_from_outside_is_reported_as_exited() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust_and_approve(&plan);
    let (session, recorder) = f.start(&plan);

    kill(
        Pid::from_raw(session.pid().unwrap() as i32),
        Signal::SIGKILL,
    )
    .unwrap();
    let exit = recorder.wait_for_exit();
    assert!(exit.signal.is_some(), "{exit:?}");
    assert!(matches!(
        f.runtime.sessions()[0].state,
        SessionState::Exited(_)
    ));
}

#[test]
fn opening_another_workspace_stops_the_previous_workspaces_agents() {
    let mut f = fixture();
    let here = f.plan(&f.workspace);
    let there = f.plan(&f.other);
    f.trust_and_approve(&here);
    f.trust_and_approve(&there);
    let (old, _) = f.start(&here);
    let (current, _) = f.start(&there);

    assert_eq!(f.runtime.stop_outside(&f.sessions, &f.other), 1);
    assert!(wait_until_gone(old.pid().unwrap()));
    assert!(alive(current.pid().unwrap()));
    assert_eq!(f.runtime.sessions().len(), 1);
    f.sessions.shutdown(Duration::from_millis(500));
}

#[test]
fn removing_trust_can_stop_the_folders_agents() {
    let mut f = fixture();
    let here = f.plan(&f.workspace);
    let there = f.plan(&f.other);
    f.trust_and_approve(&here);
    f.trust_and_approve(&there);
    let (stopped, _) = f.start(&here);
    let (kept, _) = f.start(&there);

    assert_eq!(f.runtime.stop_in(&f.sessions, &f.workspace), 1);
    assert!(wait_until_gone(stopped.pid().unwrap()));
    assert!(alive(kept.pid().unwrap()));
    f.sessions.shutdown(Duration::from_millis(500));
}

#[test]
fn a_folder_without_git_takes_one_agent_at_a_time() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust_and_approve(&plan);
    let (first, _, first_id) = f.start_session(&plan);

    // No isolation: a second agent would share the folder with the first.
    assert_eq!(
        f.runtime.create(&plan, f.workspace.clone(), None),
        Err(RunError::SharedBusy)
    );

    // Once the first has stopped, the folder is free again.
    f.runtime.stop(&f.sessions, first_id).unwrap();
    assert!(first.wait_for_exit(TIMEOUT));
    let (second, _) = f.start(&plan);
    assert!(second.pid().is_some());
    f.sessions.shutdown(Duration::from_millis(500));
}

#[test]
fn restarting_runs_the_agent_again_in_the_same_session() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust_and_approve(&plan);
    let (first, recorder, id) = f.start_session(&plan);
    first.write(b"quit\n".to_vec()).unwrap();
    recorder.wait_for_exit();

    let (second, recorder) = f.run(id, &plan);
    assert_ne!(first.id(), second.id());
    assert!(
        recorder
            .output()
            .contains(&format!("cwd={}", f.workspace.display()))
    );
    assert_eq!(f.runtime.get(id).unwrap().terminal, Some(second.id()));
    // Not twice at once.
    let again = authorize(&plan, &f.trust, &f.approvals).unwrap();
    assert!(matches!(
        f.runtime
            .run(&f.sessions, id, again, SIZE, Arc::new(Recorder::default())),
        Err(RunError::AlreadyRunning)
    ));
    f.sessions.shutdown(Duration::from_millis(500));
}

#[test]
fn a_session_runs_only_the_agent_and_workspace_it_was_made_for() {
    let mut f = fixture();
    let here = f.plan(&f.workspace);
    let there = f.plan(&f.other);
    f.trust_and_approve(&here);
    f.trust_and_approve(&there);
    let id = f.runtime.create(&here, f.workspace.clone(), None).unwrap();

    let other = authorize(&there, &f.trust, &f.approvals).unwrap();
    assert_eq!(
        f.runtime
            .run(&f.sessions, id, other, SIZE, Arc::new(Recorder::default()))
            .err(),
        Some(RunError::Mismatch)
    );
    // And a session's directory must be its workspace.
    assert_eq!(
        f.runtime.create(&here, f.other.clone(), None),
        Err(RunError::OutsideWorkspace)
    );
}

/// Stands in for `claude`: reports whether it would take itself for a child
/// session, and what else it got.
const FAKE_CLAUDE: &str = r#"#!/bin/sh
trap 'echo "interrupted"; exit 130' INT
echo "cwd=$(pwd)"
echo "child-session=${CLAUDE_CODE_CHILD_SESSION-absent}"
echo "claudecode=${CLAUDECODE-absent}"
echo "profile=${FROM_PROFILE-absent}"
echo "path=$PATH"
if [ -t 0 ] && [ -t 1 ]; then echo "tty=yes"; else echo "tty=no"; fi
echo "ready"
while read -r line; do echo "got:$line"; done
"#;

fn claude_code() -> AgentDefinition {
    x8ai_agents::builtin()
        .into_iter()
        .find(|a| a.id.as_str() == "claude-code")
        .unwrap()
}

#[test]
fn the_child_session_marker_is_the_only_variable_left_out_of_an_agents_environment() {
    let f = fixture();
    let claude = f.bin.join("claude");
    fs::write(&claude, FAKE_CLAUDE).unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    // The login environment of an app started from inside a Claude Code session.
    let mut inherited = f.environment();
    inherited.push(("CLAUDE_CODE_CHILD_SESSION".into(), "1".into()));
    inherited.push(("CLAUDECODE".into(), "1".into()));
    let launch = plan(&claude_code(), &inherited, &f.workspace).unwrap();

    assert_eq!(launch.program, claude);
    assert_eq!(launch.workspace, f.workspace);
    let expected: Vec<_> = inherited
        .iter()
        .filter(|(name, _)| name != "CLAUDE_CODE_CHILD_SESSION")
        .cloned()
        .collect();
    assert_eq!(
        launch.env, expected,
        "nothing else is added, removed or reordered"
    );
    // It is not what an approval covers, so approvals are unchanged by it.
    let without = plan(&claude_code(), &f.environment(), &f.workspace).unwrap();
    assert_eq!(launch.approval(), without.approval());
}

#[test]
fn claude_code_launched_by_the_app_is_not_a_child_of_the_session_that_started_the_app() {
    let mut f = fixture();
    let claude = f.bin.join("claude");
    fs::write(&claude, FAKE_CLAUDE).unwrap();
    fs::set_permissions(&claude, fs::Permissions::from_mode(0o755)).unwrap();
    // A login shell that inherited Claude Code's markers from the app, as it does
    // when the app was started from a Claude Code session, and a profile that
    // puts `claude` on the PATH.
    let home = f.workspace.parent().unwrap().join("home");
    fs::create_dir(&home).unwrap();
    fs::write(
        home.join(".profile"),
        format!(
            "export PATH=\"{}:$PATH\"\nexport FROM_PROFILE=yes\n",
            f.bin.display()
        ),
    )
    .unwrap();
    let shell = f.bin.join("inheriting-shell");
    fs::write(
        &shell,
        "#!/bin/sh\nexport CLAUDE_CODE_CHILD_SESSION=1 CLAUDECODE=1\nexec /bin/sh \"$@\"\n",
    )
    .unwrap();
    fs::set_permissions(&shell, fs::Permissions::from_mode(0o755)).unwrap();
    let login = x8ai_agents::environment::resolve(&shell, &home, TIMEOUT).unwrap();
    let var = |name| x8ai_agents::environment::var(&login, name);
    assert_eq!(var("CLAUDE_CODE_CHILD_SESSION"), Some("1"), "inherited");

    let launch = plan(&claude_code(), &login, &f.workspace).unwrap();
    assert_eq!(launch.program, claude, "found on the login PATH");
    // Still only with the folder trusted and the agent approved.
    assert!(matches!(
        authorize(&launch, &f.trust, &f.approvals),
        Err(Denied::Untrusted(_))
    ));
    f.trust.set(&launch.workspace, true).unwrap();
    assert!(matches!(
        authorize(&launch, &f.trust, &f.approvals),
        Err(Denied::NotApproved { .. })
    ));
    f.approvals.approve(&launch.approval()).unwrap();
    let (session, recorder) = f.start(&launch);

    let output = recorder.output();
    assert!(output.contains("child-session=absent"), "{output}");
    assert!(
        output.contains(&format!("cwd={}", f.workspace.display())),
        "{output}"
    );
    assert!(
        output.contains("claudecode=1"),
        "other variables are left alone: {output}"
    );
    assert!(output.contains("profile=yes"), "{output}");
    assert!(
        output.contains(&format!("path={}", var("PATH").unwrap())),
        "the login PATH: {output}"
    );
    assert!(output.contains("tty=yes"), "{output}");
    // The PTY behaves as for any agent.
    session.write(b"hello\n".to_vec()).unwrap();
    recorder.wait_for("got:hello");
    session.write(vec![0x03]).unwrap();
    recorder.wait_for("interrupted");
    assert_eq!(recorder.wait_for_exit().code, 130);
}
