//! The real Claude Code with an MCP server the app starts. Not run by default:
//!
//! ```sh
//! cargo test -p x8ai-mcp --test live_claude_mcp -- --ignored --nocapture
//! ```
//!
//! Everything is disposable: a temporary folder as the workspace, the test
//! server (`fixtures/echo-server.sh`), stores and sockets in the same folder, a
//! secret in memory, decoy provider keys in the login environment. Claude Code
//! is never sent a prompt. It connects to its MCP servers once the folder is
//! trusted in its own trust prompt, so the test answers that prompt for the
//! disposable folder (Claude Code records the answer in its own configuration).
//! Nothing the test sets up is left running.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use x8ai_agents::adapter::{AgentMcpServer, AgentMcpTransport, attach_mcp};
use x8ai_agents::environment::{RESOLVE_TIMEOUT, resolve};
use x8ai_agents::{AgentRuntime, authorize as authorize_agent, builtin, plan};
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{
    McpEnvSource, McpEnvVar, McpScope, McpServer, McpServerState, McpServerTransport,
};
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_mcp::{
    Approvals, Launch, Limits, McpRuntime, authorize, environment, prepare, secret_account,
};
use x8ai_pty::{SessionEvents, Sessions, user_shell};
use x8ai_secrets::{MemoryStore, SecretStore, SecretValue};
use x8ai_workspace::{ApprovalStore, TrustStore};

const SERVER: &str = include_str!("fixtures/echo-server.sh");
/// Made when the test runs, so no file can hold it by coincidence (this test's
/// source, a transcript): a match anywhere means it leaked.
fn test_secret() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("x8ai-live-invalid-{:x}{:x}", std::process::id(), nanos)
}

#[derive(Default)]
struct Screen {
    state: Mutex<(Vec<u8>, Option<TerminalExit>)>,
    changed: Condvar,
}

impl Screen {
    /// What was drawn, without escape sequences or spaces.
    fn text(&self) -> String {
        let raw = String::from_utf8_lossy(&self.state.lock().unwrap().0).into_owned();
        let mut text = String::new();
        let mut chars = raw.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\u{1b}' {
                if chars.next_if_eq(&'[').is_some() {
                    while chars
                        .next()
                        .is_some_and(|c| !c.is_ascii_alphabetic() && c != '~')
                    {}
                } else {
                    chars.next();
                }
            } else if !c.is_control() && !c.is_whitespace() {
                text.push(c);
            }
        }
        text
    }
}

impl SessionEvents for Screen {
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

fn run(program: &str, args: &[&str]) -> String {
    let output = Command::new(program).args(args).output().unwrap();
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
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

fn wait_for(what: &str, timeout: Duration, mut check: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while !check() {
        if Instant::now() > deadline {
            println!("   (gave up waiting for {what})");
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    true
}

fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file()
                && fs::read(&path)
                    .is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle.as_bytes()))
            {
                found.push(path);
            }
        }
    }
    found
}

#[test]
#[ignore = "needs Claude Code installed; run by hand"]
fn claude_code_uses_an_mcp_server_the_app_starts_and_nothing_leaks() {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let (workspace, bin, out) = (
        base.join("disposable-project"),
        base.join("bin"),
        base.join("server-out"),
    );
    for dir in [&workspace, &bin, &out] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(workspace.join("README.md"), "# disposable test project\n").unwrap();
    fs::write(bin.join("x8ai-echo-server"), SERVER).unwrap();
    fs::set_permissions(
        bin.join("x8ai-echo-server"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();

    // The login environment, as the app reads it, plus provider keys the shell
    // might carry: none may reach the MCP server.
    let home = std::env::home_dir().unwrap();
    let mut login = resolve(&PathBuf::from(user_shell()), &home, RESOLVE_TIMEOUT).unwrap();
    login.retain(|(n, _)| {
        !n.starts_with("ANTHROPIC_") && !n.starts_with("CLAUDE_CODE_") && n != "CLAUDECODE"
    });
    for (name, value) in [
        ("OPENAI_API_KEY", "sk-openai-login-shell-invalid"),
        ("OPENROUTER_API_KEY", "sk-or-login-shell-invalid"),
        ("GITHUB_TOKEN", "ghp-login-shell-invalid"),
        ("AWS_SECRET_ACCESS_KEY", "aws-login-shell-invalid"),
    ] {
        login.retain(|(n, _)| n != name);
        login.push((name.to_owned(), value.to_owned()));
    }

    // 1-3. A server in the registry's form, its secret saved, the folder trusted.
    let server = McpServer {
        id: IntegrationId::new("echo").unwrap(),
        name: "Echo (test)".into(),
        description: String::new(),
        transport: McpServerTransport::Stdio {
            command: bin.join("x8ai-echo-server").display().to_string(),
            args: vec![out.display().to_string()],
        },
        env: vec![McpEnvVar {
            name: "TEST_TOKEN".into(),
            source: McpEnvSource::Secret,
        }],
        enabled: true,
        scope: McpScope::Global,
    };
    let secret = test_secret();
    let secrets = MemoryStore::default();
    secrets
        .set(
            &secret_account("echo", "TEST_TOKEN"),
            &SecretValue::new(&secret).unwrap(),
        )
        .unwrap();
    let (mut trust, _) = TrustStore::load(base.join("data/trusted.json"));
    let (mut approvals, _) = Approvals::load(base.join("data/mcp-approvals.json"));
    let prepared = [prepare(&server, x8ai_agents::environment::var(&login, "PATH")).unwrap()];
    assert!(
        authorize(&workspace, &prepared, &trust, &approvals).is_err(),
        "untrusted"
    );
    trust.set(&workspace, true).unwrap();
    assert!(
        authorize(&workspace, &prepared, &trust, &approvals).is_err(),
        "not approved"
    );
    approvals
        .approve(&workspace, &[("echo", &prepared[0].material)])
        .unwrap();
    let authorized = authorize(&workspace, &prepared, &trust, &approvals).unwrap();
    println!(
        "1-5. server added, secret saved, folder trusted, server approved (refused before each)"
    );

    // Its socket: nothing runs yet.
    let runtime = McpRuntime::new(base.join("home/.x8ai/mcp"), Limits::default());
    let env = environment(&server, &login, &secrets).unwrap();
    let launch = Launch::new(&prepared[0], env, workspace.clone()).unwrap();
    let endpoints = runtime.start(1, &authorized, vec![launch]).unwrap();
    assert_eq!(runtime.states(1)[0].1, McpServerState::Waiting);

    // 6. Claude Code with its own model configuration and the server, for this
    // session only. (With an app-configured key Claude Code would first ask
    // whether to use it; the MCP server's environment does not depend on it.)
    let claude = builtin()
        .into_iter()
        .find(|a| a.id.as_str() == "claude-code")
        .unwrap();
    let own = plan(&claude, &login, &workspace).expect("Claude Code is installed");
    let agent_plan = attach_mcp(
        own,
        &claude.capabilities.mcp_transports,
        &[AgentMcpServer {
            id: IntegrationId::new("echo").unwrap(),
            transport: AgentMcpTransport::Stdio {
                command: PathBuf::from(env!("CARGO_BIN_EXE_x8ai-mcp-bridge")),
                args: vec![endpoints[0].socket.display().to_string()],
            },
        }],
    )
    .unwrap();
    let (mut agent_approvals, _) = ApprovalStore::load(base.join("data/approvals.json"));
    agent_approvals.approve(&agent_plan.approval()).unwrap();
    let agents = AgentRuntime::default();
    let sessions = Sessions::default();
    let session = agents.create(&agent_plan, workspace.clone(), None).unwrap();
    let screen = Arc::new(Screen::default());
    let pty = agents
        .run(
            &sessions,
            session,
            authorize_agent(&agent_plan, &trust, &agent_approvals).unwrap(),
            TerminalSize {
                cols: 120,
                rows: 36,
            },
            screen.clone(),
        )
        .unwrap();
    let claude_pid = pty.pid().unwrap();
    runtime.set_owner(1, claude_pid);
    println!("6. Claude Code started (pid {claude_pid}) with --mcp-config for this session");
    // Nothing starts while Claude Code waits for its own folder-trust answer.
    let asked = wait_for(
        "Claude Code's trust prompt",
        Duration::from_secs(20),
        || screen.text().contains("trustthisfolder"),
    );
    assert!(
        asked,
        "screen: {}",
        screen.text().chars().take(400).collect::<String>()
    );
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(runtime.states(1)[0].1, McpServerState::Waiting);
    // "Yes, I trust this folder" (the second choice), for the disposable folder.
    pty.write(b"\x1b[B".to_vec()).unwrap();
    std::thread::sleep(Duration::from_millis(400));
    pty.write(b"\r".to_vec()).unwrap();
    println!("   answered Claude Code's own folder-trust prompt for the disposable folder");

    // 7-8. It connects, through the bridge, and the app starts the server.
    let started = wait_for("the server to start", Duration::from_secs(30), || {
        matches!(
            runtime.states(1)[0].1,
            McpServerState::Running { .. } | McpServerState::Exited { .. }
        )
    });
    let state = runtime.states(1)[0].1.clone();
    println!("7. MCP server state after Claude Code started: {state:?}");
    if !started {
        let tree = run("ps", &["-ax", "-o", "pid=,ppid=,command="]);
        let children: Vec<&str> = tree
            .lines()
            .filter(|l| l.split_whitespace().nth(1) == Some(claude_pid.to_string().as_str()))
            .collect();
        println!("   Claude Code's children: {children:#?}");
        let _ = agents.stop(&sessions, session);
    }
    assert!(started, "Claude Code did not connect to the server");
    let pid_files = || -> Vec<PathBuf> {
        fs::read_dir(&out)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.to_string_lossy().contains("pid."))
            .collect()
    };
    assert!(wait_for(
        "the server's first output",
        Duration::from_secs(10),
        || !pid_files().is_empty()
    ));
    let pid_files = pid_files();
    let server_pid: u32 = fs::read_to_string(&pid_files[0])
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let names_file = fs::read_dir(&out)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.to_string_lossy().contains("env-names."))
        .unwrap();
    let names: Vec<String> = fs::read_to_string(names_file)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect();
    println!("8. the server's variables (names only): {names:?}");
    assert!(names.contains(&"TEST_TOKEN".to_owned()) && names.contains(&"PATH".to_owned()));
    for leaked in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "GITHUB_TOKEN",
        "AWS_SECRET_ACCESS_KEY",
        "CLAUDE_CODE_ENTRYPOINT",
    ] {
        assert!(
            !names.contains(&leaked.to_owned()),
            "{leaked} reached the MCP server"
        );
    }
    // It spoke MCP with it: the handshake, then the tool list.
    let methods = || -> Vec<String> {
        fs::read_dir(&out)
            .unwrap()
            .flatten()
            .filter(|e| e.path().to_string_lossy().contains("methods."))
            .flat_map(|e| {
                fs::read_to_string(e.path())
                    .unwrap_or_default()
                    .lines()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            })
            .collect()
    };
    wait_for("the tool list request", Duration::from_secs(15), || {
        methods().iter().any(|m| m == "tools/list")
    });
    println!("   Claude Code asked the server for: {:?}", methods());
    assert!(
        methods().iter().any(|m| m == "initialize") && methods().iter().any(|m| m == "tools/list")
    );
    let args = run("ps", &["-ww", "-o", "args=", "-p", &claude_pid.to_string()]);
    assert!(
        args.contains("--mcp-config")
            && args.contains("x8ai-echo")
            && !args.contains(secret.as_str()),
        "{args}"
    );
    let tree = run("ps", &["-ax", "-o", "pid=,ppid=,command="]);
    let bridge: Vec<&str> = tree
        .lines()
        .filter(|l| l.contains("x8ai-mcp-bridge"))
        .collect();
    println!("   the agent's MCP child: {bridge:?}");
    println!("   Claude Code's argv carries the bridge and a socket, not the server or its secret");

    // 9. The session ends (hung up, as Stop does: no exit is reported to the
    // runtime): the run notices its agent is gone and ends too.
    agents.stop(&sessions, session).unwrap();
    assert!(gone_within(claude_pid, Duration::from_secs(5)));
    assert!(
        gone_within(server_pid, Duration::from_secs(5)),
        "the MCP server stopped"
    );
    assert!(wait_for("the run to end", Duration::from_secs(5), || {
        runtime.states(1).is_empty()
    }));
    let leftovers = run("pgrep", &["-f", &base.display().to_string()]);
    assert!(leftovers.is_empty(), "left running: {leftovers}");
    assert!(!endpoints[0].socket.exists());
    println!(
        "9. session stopped: Claude Code, the bridge and the MCP server are gone; sockets removed"
    );

    // 12. The secret was nowhere but in the server's environment.
    assert_eq!(files_containing(&base, &secret), Vec::<PathBuf>::new());
    assert!(files_containing(&home.join(".claude"), &secret).is_empty());
    let claude_json = fs::read_to_string(home.join(".claude.json")).unwrap_or_default();
    assert!(!claude_json.contains(secret.as_str()));
    println!("12. secret not in the workspace, the stores, ~/.claude or ~/.claude.json");

    // 10-11. Restart: a new runtime starts nothing on its own.
    let restarted = McpRuntime::new(base.join("home/.x8ai/mcp"), Limits::default());
    restarted.sweep();
    std::thread::sleep(Duration::from_millis(500));
    assert!(restarted.running_pids().is_empty() && restarted.states(1).is_empty());
    let pids_after = fs::read_dir(&out)
        .unwrap()
        .flatten()
        .filter(|e| e.path().to_string_lossy().contains("pid."))
        .count();
    assert_eq!(pids_after, pid_files.len(), "nothing started again");
    println!("10-11. a restarted runtime starts no server");
}
