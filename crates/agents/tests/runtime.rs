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
use x8ai_agents::{AgentRuntime, Denied, LaunchPlan, RunState, authorize, plan};
use x8ai_core::agent::AgentDefinition;
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

    fn start(&self, plan: &LaunchPlan) -> (Arc<x8ai_pty::Session>, Arc<Recorder>) {
        let recorder = Arc::new(Recorder::default());
        let authorized = authorize(plan, &self.trust, &self.approvals).expect("authorized");
        let session = self
            .runtime
            .start(&self.sessions, authorized, SIZE, recorder.clone())
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

    let status = f.runtime.status(session.id()).unwrap();
    assert_eq!(status.state, RunState::Running);
    assert_eq!(status.workspace, f.workspace);
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
    assert_eq!(
        f.runtime.status(session.id()).unwrap().state,
        RunState::Exited
    );
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
    let (session, _recorder) = f.start(&plan);
    let pid = session.pid().unwrap();
    // The agent is busy with a child process when the terminal closes.
    session.write(b"wait\n".to_vec()).unwrap();
    std::thread::sleep(Duration::from_millis(200));

    f.runtime.stop(&f.sessions, session.id()).unwrap();
    assert!(wait_until_gone(pid), "the agent outlived its terminal");
    assert!(f.sessions.get(session.id()).is_err());
    assert!(f.runtime.status(session.id()).is_none());
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
    assert_eq!(
        f.runtime.status(session.id()).unwrap().state,
        RunState::Exited
    );
    assert!(!f.runtime.any_running());
}

#[test]
fn quitting_the_app_ends_every_agent() {
    let mut f = fixture();
    let plan = f.plan(&f.workspace);
    f.trust_and_approve(&plan);
    let (first, _) = f.start(&plan);
    let (second, _) = f.start(&plan);
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
    assert_eq!(
        f.runtime.status(session.id()).unwrap().state,
        RunState::Exited
    );
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
