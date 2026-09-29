//! Session-owned stdio MCP servers, with real processes: the test server
//! (`fixtures/echo-server.sh`) behind the real bridge (`x8ai-mcp-bridge`), which
//! plays the part of what the agent starts. The test process is the "agent".

use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{
    McpEnvSource, McpEnvVar, McpScope, McpServer, McpServerState, McpServerTransport,
};
use x8ai_mcp::runtime::RuntimeError;
use x8ai_mcp::{
    Approvals, Launch, Limits, McpRuntime, Prepared, authorize, environment, prepare,
    secret_account,
};
use x8ai_secrets::{MemoryStore, SecretStore, SecretValue};
use x8ai_workspace::TrustStore;

const SERVER: &str = include_str!("fixtures/echo-server.sh");
const INITIALIZE: &str = r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}"#;
const TOOLS: &str = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    workspace: PathBuf,
    out: PathBuf,
    login: Vec<(String, String)>,
    trust: TrustStore,
    approvals: Approvals,
    secrets: MemoryStore,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (workspace, bin, out) = (root.join("project"), root.join("bin"), root.join("out"));
    for dir in [&workspace, &bin, &out] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(bin.join("echo-server"), SERVER).unwrap();
    fs::set_permissions(bin.join("echo-server"), fs::Permissions::from_mode(0o755)).unwrap();
    let login = vec![
        (
            "PATH".to_owned(),
            format!("{}:/usr/bin:/bin", bin.display()),
        ),
        ("HOME".to_owned(), root.display().to_string()),
        (
            "ANTHROPIC_API_KEY".to_owned(),
            "sk-ant-login-shell-invalid".to_owned(),
        ),
        (
            "OPENAI_API_KEY".to_owned(),
            "sk-openai-login-shell-invalid".to_owned(),
        ),
    ];
    let (mut trust, _) = TrustStore::load(root.join("data/trusted.json"));
    trust.set(&workspace, true).unwrap();
    let (approvals, _) = Approvals::load(root.join("data/mcp-approvals.json"));
    Fixture {
        _temp: temp,
        root,
        workspace,
        out,
        login,
        trust,
        approvals,
        secrets: MemoryStore::default(),
    }
}

fn server(id: &str, args: &[&str], secret: Option<&str>) -> McpServer {
    McpServer {
        id: IntegrationId::new(id).unwrap(),
        name: id.to_uppercase(),
        description: String::new(),
        transport: McpServerTransport::Stdio {
            command: "echo-server".into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        },
        env: secret
            .map(|name| McpEnvVar {
                name: name.into(),
                source: McpEnvSource::Secret,
            })
            .into_iter()
            .collect(),
        enabled: true,
        scope: McpScope::Global,
    }
}

impl Fixture {
    fn out_arg(&self) -> String {
        self.out.display().to_string()
    }

    fn runtime(&self, limits: Limits) -> McpRuntime {
        McpRuntime::new(self.root.join("home/.x8ai/mcp"), limits)
    }

    fn prepared(&mut self, servers: &[McpServer]) -> Vec<Prepared> {
        let path = self.login[0].1.clone();
        let prepared: Vec<Prepared> = servers
            .iter()
            .map(|s| prepare(s, Some(&path)).unwrap())
            .collect();
        let pairs: Vec<(&str, &x8ai_mcp::Material)> = prepared
            .iter()
            .map(|p| (p.server.id.as_str(), &p.material))
            .collect();
        self.approvals.approve(&self.workspace, &pairs).unwrap();
        prepared
    }

    /// Starts `servers` for `session` the way the app does: approved, authorized,
    /// with their environments.
    fn start(&mut self, runtime: &McpRuntime, session: u32, servers: &[McpServer]) -> Vec<PathBuf> {
        let prepared = self.prepared(servers);
        let authorized =
            authorize(&self.workspace, &prepared, &self.trust, &self.approvals).unwrap();
        let launches = prepared
            .iter()
            .map(|p| {
                let env = environment(&p.server, &self.login, &self.secrets).unwrap();
                Launch::new(p, env, self.workspace.clone()).unwrap()
            })
            .collect();
        runtime
            .start(session, &authorized, launches)
            .unwrap()
            .into_iter()
            .map(|e| e.socket)
            .collect()
    }

    fn files(&self, prefix: &str) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = fs::read_dir(&self.out)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(prefix))
            .collect();
        files.sort();
        files
    }

    /// The first server's process id, once it has started.
    fn wait_for_pid(&self) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(pid) = self.pids("pid.").first() {
                return *pid;
            }
            assert!(Instant::now() < deadline, "the server did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn pids(&self, prefix: &str) -> Vec<u32> {
        self.files(prefix)
            .iter()
            .map(|p| fs::read_to_string(p).unwrap().trim().parse().unwrap())
            .collect()
    }
}

/// The bridge, as the agent would start it.
struct Client {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
}

fn connect(socket: &Path) -> Client {
    let mut child = Command::new(env!("CARGO_BIN_EXE_x8ai-mcp-bridge"))
        .arg(socket)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, lines) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(line);
        }
    });
    Client {
        stdin: child.stdin.take(),
        child,
        lines,
    }
}

impl Client {
    fn ask(&mut self, message: &str) -> Option<String> {
        let stdin = self.stdin.as_mut()?;
        writeln!(stdin, "{message}").ok()?;
        stdin.flush().ok()?;
        self.lines.recv_timeout(Duration::from_secs(10)).ok()
    }

    fn exits_within(&mut self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.child.try_wait().unwrap().is_some() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn alive(pid: u32) -> bool {
    // Signal 0: exists (a zombie counts as gone once reaped by its parent).
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(i32::try_from(pid).unwrap()),
        None,
    )
    .is_ok()
}

fn gone_within(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while alive(pid) {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    true
}

fn wait_for_state(
    runtime: &McpRuntime,
    session: u32,
    index: usize,
    done: impl Fn(&McpServerState) -> bool,
) -> McpServerState {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = runtime.states(session)[index].1.clone();
        if done(&state) || Instant::now() > deadline {
            return state;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn me() -> u32 {
    std::process::id()
}

#[test]
fn a_server_starts_only_when_the_sessions_agent_connects() {
    let mut f = fixture();
    let runtime = f.runtime(Limits::default());
    f.secrets
        .set(
            &secret_account("echo", "TEST_TOKEN"),
            &SecretValue::new("test-token-0000-invalid").unwrap(),
        )
        .unwrap();
    let out = f.out_arg();
    let echo = server(
        "echo",
        &[&out, "normal", "; rm -rf ~", "$(id)", "a b"],
        Some("TEST_TOKEN"),
    );
    let sockets = f.start(&runtime, 1, &[echo]);

    // Ready, but nothing runs: no agent has connected.
    assert_eq!(runtime.states(1)[0].1, McpServerState::Waiting);
    std::thread::sleep(Duration::from_millis(200));
    assert!(f.pids("pid.").is_empty());

    runtime.set_owner(1, me());
    let mut client = connect(&sockets[0]);
    let answer = client
        .ask(INITIALIZE)
        .expect("an answer through the bridge");
    assert!(
        answer.contains(r#""serverInfo":{"name":"x8ai-echo""#),
        "{answer}"
    );
    assert!(client.ask(TOOLS).unwrap().contains(r#""name":"echo""#));
    let pid = f.pids("pid.")[0];
    assert_eq!(runtime.states(1)[0].1, McpServerState::Running { pid });

    // Exactly the approved argv, each argument as it is: nothing interpreted.
    let args = fs::read_to_string(&f.files("args.")[0]).unwrap();
    assert_eq!(
        args.lines().collect::<Vec<_>>(),
        [out.as_str(), "normal", "; rm -rf ~", "$(id)", "a b"]
    );
    // The base, its secret, and not the login shell's provider keys.
    let names = fs::read_to_string(&f.files("env-names.")[0]).unwrap();
    let names: Vec<&str> = names
        .lines()
        .filter(|n| !["PWD", "SHLVL", "_", "OLDPWD"].contains(n))
        .collect();
    assert_eq!(names, ["HOME", "PATH", "TEST_TOKEN"]);
    assert_eq!(runtime.running_pids(), [pid]);

    // The secret is in the server's environment only: not in the session's
    // directory, not beside the sockets, not in the stores.
    for dir in [&f.workspace, &f.root.join("home"), &f.root.join("data")] {
        assert_eq!(
            files_containing(dir, "test-token-0000-invalid"),
            Vec::<PathBuf>::new(),
            "{}",
            dir.display()
        );
    }
}

/// Every file under `dir` whose bytes contain `needle` (sockets are skipped).
fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file()
                && fs::read(&path)
                    .unwrap()
                    .windows(needle.len())
                    .any(|w| w == needle.as_bytes())
            {
                found.push(path);
            }
        }
    }
    found
}

#[test]
fn a_connection_from_outside_the_session_starts_nothing() {
    let mut f = fixture();
    let runtime = f.runtime(Limits {
        owner_wait: Duration::from_millis(300),
        ..Limits::default()
    });
    let out = f.out_arg();
    let sockets = f.start(&runtime, 1, &[server("echo", &[&out], None)]);

    // Before the agent is known: refused after the wait.
    let mut early = connect(&sockets[0]);
    assert!(
        early.exits_within(Duration::from_secs(3)),
        "the bridge is disconnected"
    );

    // Another process as the agent: this test process is not one of its descendants.
    let mut agent = Command::new("/bin/sleep").arg("30").spawn().unwrap();
    runtime.set_owner(1, agent.id());
    let mut stranger = connect(&sockets[0]);
    assert!(stranger.exits_within(Duration::from_secs(3)));
    assert!(f.pids("pid.").is_empty(), "no server was started");
    assert_eq!(runtime.states(1)[0].1, McpServerState::Waiting);
    let _ = agent.kill();
    let _ = agent.wait();
}

#[test]
fn stopping_the_session_ends_the_server_and_everything_it_started() {
    let mut f = fixture();
    let runtime = f.runtime(Limits::default());
    let out = f.out_arg();
    let sockets = f.start(&runtime, 1, &[server("echo", &[&out, "orphan"], None)]);
    runtime.set_owner(1, me());
    let mut client = connect(&sockets[0]);
    assert!(client.ask(INITIALIZE).is_some());
    let (server_pid, child_pid) = (f.pids("pid.")[0], f.pids("child.")[0]);
    assert!(alive(server_pid) && alive(child_pid));
    let run_dir = sockets[0].parent().unwrap().to_owned();
    assert_eq!(
        fs::metadata(&run_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&sockets[0]).unwrap().permissions().mode() & 0o777,
        0o600
    );

    runtime.stop(1);
    assert!(gone_within(server_pid, Duration::from_secs(5)));
    assert!(gone_within(child_pid, Duration::from_secs(5)), "no orphan");
    assert!(
        client.exits_within(Duration::from_secs(5)),
        "the agent sees the server go"
    );
    assert!(!run_dir.exists(), "sockets removed");
    assert!(runtime.states(1).is_empty());
}

#[test]
fn the_server_ends_when_the_agent_disconnects() {
    let mut f = fixture();
    let runtime = f.runtime(Limits::default());
    let out = f.out_arg();
    let sockets = f.start(&runtime, 1, &[server("echo", &[&out], None)]);
    runtime.set_owner(1, me());
    let mut client = connect(&sockets[0]);
    assert!(client.ask(INITIALIZE).is_some());
    let pid = f.pids("pid.")[0];
    drop(client.stdin.take());
    assert!(gone_within(pid, Duration::from_secs(5)));
    assert_eq!(
        wait_for_state(&runtime, 1, 0, |s| matches!(
            s,
            McpServerState::Exited { .. }
        )),
        McpServerState::Exited { code: Some(0) }
    );
}

#[test]
fn a_crash_is_reported_redacted_and_restarts_are_bounded() {
    let mut f = fixture();
    let runtime = f.runtime(Limits {
        max_starts: 3,
        ..Limits::default()
    });
    f.secrets
        .set(
            &secret_account("crashy", "TEST_TOKEN"),
            &SecretValue::new("crash-token-0000-invalid").unwrap(),
        )
        .unwrap();
    let out = f.out_arg();
    let sockets = f.start(
        &runtime,
        1,
        &[server("crashy", &[&out, "crash"], Some("TEST_TOKEN"))],
    );
    runtime.set_owner(1, me());

    let mut first = connect(&sockets[0]);
    assert!(
        first.exits_within(Duration::from_secs(5)),
        "the agent sees the crash"
    );
    let state = wait_for_state(&runtime, 1, 0, |s| {
        matches!(s, McpServerState::Failed { .. })
    });
    let McpServerState::Failed { message } = state else {
        panic!("{state:?}")
    };
    assert_eq!(
        message,
        "exited with code 3: fatal: the token <redacted> was rejected"
    );

    // The agent may reconnect, up to the limit; then nothing starts.
    for _ in 0..4 {
        let mut again = connect(&sockets[0]);
        assert!(again.exits_within(Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(100));
    }
    assert_eq!(f.pids("pid.").len(), 3);
    let McpServerState::Failed { message } = runtime.states(1)[0].1.clone() else {
        panic!()
    };
    assert!(message.contains("started it 3 times"), "{message}");
}

#[test]
fn a_server_that_never_answers_is_stopped_after_the_startup_timeout() {
    let mut f = fixture();
    let runtime = f.runtime(Limits {
        startup_timeout: Duration::from_millis(300),
        ..Limits::default()
    });
    let out = f.out_arg();
    let sockets = f.start(&runtime, 1, &[server("silent", &[&out, "silent"], None)]);
    runtime.set_owner(1, me());
    let mut client = connect(&sockets[0]);
    let pid = f.wait_for_pid();
    // Connected but not asked anything yet: the clock starts with the first request.
    std::thread::sleep(Duration::from_millis(600));
    assert!(alive(pid));
    assert!(client.ask(INITIALIZE).is_none(), "no answer");
    assert!(gone_within(pid, Duration::from_secs(5)));
    let McpServerState::Failed { message } = wait_for_state(&runtime, 1, 0, |s| {
        matches!(s, McpServerState::Failed { .. })
    }) else {
        panic!()
    };
    assert!(
        message.starts_with("did not answer within 0.3 s"),
        "{message}"
    );
}

#[test]
fn sessions_and_servers_are_isolated_and_quitting_leaves_nothing_running() {
    let mut f = fixture();
    let runtime = f.runtime(Limits::default());
    for (server, value) in [
        ("one", "one-token-0000-invalid"),
        ("two", "two-token-0000-invalid"),
    ] {
        f.secrets
            .set(
                &secret_account(server, &format!("{}_TOKEN", server.to_uppercase())),
                &SecretValue::new(value).unwrap(),
            )
            .unwrap();
    }
    let out = f.out_arg();
    let first = f.start(
        &runtime,
        1,
        &[
            server("one", &[&out], Some("ONE_TOKEN")),
            server("two", &[&out], Some("TWO_TOKEN")),
        ],
    );
    let second = f.start(&runtime, 2, &[server("one", &[&out], Some("ONE_TOKEN"))]);
    assert_ne!(
        first[0].parent(),
        second[0].parent(),
        "each session has its own sockets"
    );
    runtime.set_owner(1, me());
    runtime.set_owner(2, me());
    let mut clients: Vec<Client> = [&first[0], &first[1], &second[0]]
        .into_iter()
        .map(|s| connect(s))
        .collect();
    for client in &mut clients {
        assert!(client.ask(INITIALIZE).is_some());
    }
    let mut pids = runtime.running_pids();
    pids.sort_unstable();
    assert_eq!(pids.len(), 3);

    // Each server has its own secret only.
    let mut envs: Vec<String> = f
        .files("env-names.")
        .iter()
        .map(|p| fs::read_to_string(p).unwrap())
        .collect();
    envs.sort();
    assert_eq!(
        envs.iter()
            .filter(|e| e.contains("ONE_TOKEN") && !e.contains("TWO_TOKEN"))
            .count(),
        2
    );
    assert_eq!(
        envs.iter()
            .filter(|e| e.contains("TWO_TOKEN") && !e.contains("ONE_TOKEN"))
            .count(),
        1
    );

    // Stopping one session leaves the other's server running.
    runtime.stop(1);
    let still: Vec<u32> = runtime.running_pids();
    assert_eq!(still.len(), 1);
    assert!(
        pids.iter()
            .filter(|p| !still.contains(p))
            .all(|p| gone_within(*p, Duration::from_secs(5)))
    );

    // Quitting stops the rest.
    drop(runtime);
    assert!(gone_within(still[0], Duration::from_secs(5)));
    assert!(
        fs::read_dir(f.root.join("home/.x8ai/mcp"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
fn only_exactly_what_was_approved_can_be_started() {
    let mut f = fixture();
    let runtime = f.runtime(Limits::default());
    let out = f.out_arg();
    let approved = f.prepared(&[server("echo", &[&out], None)]);
    let authorized = authorize(&f.workspace, &approved, &f.trust, &f.approvals).unwrap();
    let mut launch = Launch::new(&approved[0], Default::default(), f.workspace.clone()).unwrap();
    launch.args.push("--allow-everything".into());
    assert!(matches!(
        runtime.start(1, &authorized, vec![launch]),
        Err(RuntimeError::NotAuthorized(_))
    ));
    assert!(runtime.states(1).is_empty());
}

#[test]
fn a_new_runtime_starts_nothing_and_sweeps_what_a_crash_left() {
    let f = fixture();
    let dir = f.root.join("home/.x8ai/mcp");
    fs::create_dir_all(dir.join("7-deadbeef")).unwrap();
    fs::write(dir.join("7-deadbeef/0.sock"), "").unwrap();
    fs::create_dir_all(dir.join("not-ours")).unwrap();
    let runtime = f.runtime(Limits::default());
    assert!(runtime.states(7).is_empty() && runtime.running_pids().is_empty());
    runtime.sweep();
    assert!(!dir.join("7-deadbeef").exists());
    assert!(
        dir.join("not-ours").exists(),
        "only what the runtime makes is removed"
    );
}

#[test]
fn a_late_stop_of_an_earlier_run_leaves_the_new_run_alone() {
    let mut f = fixture();
    let runtime = f.runtime(Limits::default());
    let out = f.out_arg();
    f.start(&runtime, 1, &[server("echo", &[&out], None)]);
    let first = runtime.run_token(1).unwrap();
    // The agent restarted: a new run replaces the first.
    let sockets = f.start(&runtime, 1, &[server("echo", &[&out], None)]);
    let second = runtime.run_token(1).unwrap();
    assert_ne!(first, second);
    // The first agent's exit arrives late.
    runtime.stop_run(1, first);
    assert_eq!(runtime.run_token(1), Some(second));
    runtime.set_owner(1, me());
    let mut client = connect(&sockets[0]);
    assert!(client.ask(INITIALIZE).is_some(), "the new run still serves");
    runtime.stop_run(1, second);
    assert!(runtime.states(1).is_empty());
}

#[test]
fn a_run_ends_when_its_agent_process_is_gone_however_it_ended() {
    let mut f = fixture();
    let runtime = f.runtime(Limits::default());
    let out = f.out_arg();
    let sockets = f.start(&runtime, 1, &[server("echo", &[&out, "orphan"], None)]);
    // The "agent" is the bridge itself: it connects, then is killed from outside,
    // as when a terminal closes. No exit is reported to the runtime.
    let mut agent = connect(&sockets[0]);
    runtime.set_owner(1, agent.child.id());
    assert!(agent.ask(INITIALIZE).is_some());
    let (server_pid, child_pid) = (f.wait_for_pid(), f.pids("child.")[0]);
    let run_dir = sockets[0].parent().unwrap().to_owned();
    let _ = agent.child.kill();
    let _ = agent.child.wait();
    assert!(gone_within(server_pid, Duration::from_secs(5)));
    assert!(gone_within(child_pid, Duration::from_secs(5)), "no orphan");
    let deadline = Instant::now() + Duration::from_secs(5);
    while (!runtime.states(1).is_empty() || run_dir.exists()) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(runtime.states(1).is_empty(), "the run is over");
    assert!(!run_dir.exists(), "its sockets are gone");
}
