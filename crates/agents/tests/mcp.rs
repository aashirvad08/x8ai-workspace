//! Giving agents MCP servers for one session through their adapters
//! (docs/mcp.md): what each agent is told, what it is never told, and that
//! nothing global changes.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use x8ai_agents::adapter::{
    AgentMcpServer, AgentMcpTransport, ConfigureError, attach_mcp, configure, mcp_support,
};
use x8ai_agents::{AgentRuntime, LaunchPlan, RunError, authorize, builtin, plan};
use x8ai_core::agent::AgentDefinition;
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::McpTransportKind;
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_pty::{SessionEvents, Sessions};
use x8ai_secrets::SecretValue;
use x8ai_workspace::{ApprovalStore, TrustStore};

const BOTH: &[McpTransportKind] = &[McpTransportKind::Stdio, McpTransportKind::StreamableHttp];

const FAKE_AGENT: &str = r#"#!/bin/sh
for arg in "$@"; do printf '%s\n' "$arg"; done > "$HOME/agent-args"
printf '%s' "${OPENCODE_CONFIG_CONTENT-}" > "$HOME/agent-opencode-config"
echo done
"#;

fn id(value: &str) -> IntegrationId {
    IntegrationId::new(value).unwrap()
}

fn stdio(server: &str) -> AgentMcpServer {
    AgentMcpServer {
        id: id(server),
        transport: AgentMcpTransport::Stdio {
            command: PathBuf::from("/Applications/x8ai Workspace.app/Contents/MacOS/x8ai-desktop"),
            args: vec![
                "--mcp-bridge".into(),
                format!("/Users/me/.x8ai/mcp/1-abcdef01/{server}.sock"),
            ],
        },
    }
}

fn remote(server: &str, url: &str) -> AgentMcpServer {
    AgentMcpServer {
        id: id(server),
        transport: AgentMcpTransport::StreamableHttp { url: url.into() },
    }
}

struct Machine {
    _temp: tempfile::TempDir,
    root: PathBuf,
    bin: PathBuf,
}

fn machine() -> Machine {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::create_dir_all(root.join("home")).unwrap();
    fs::create_dir_all(root.join("project")).unwrap();
    for program in ["claude", "opencode", "codex"] {
        fs::write(bin.join(program), FAKE_AGENT).unwrap();
        fs::set_permissions(bin.join(program), fs::Permissions::from_mode(0o755)).unwrap();
    }
    Machine {
        _temp: temp,
        root,
        bin,
    }
}

impl Machine {
    fn shell(&self) -> Vec<(String, String)> {
        vec![
            (
                "PATH".into(),
                format!("{}:/usr/bin:/bin", self.bin.display()),
            ),
            ("HOME".into(), self.root.join("home").display().to_string()),
            ("ANTHROPIC_API_KEY".into(), "sk-shell-invalid".into()),
        ]
    }

    fn plan(&self, agent: &str) -> LaunchPlan {
        let definition = builtin()
            .into_iter()
            .find(|a| a.id.as_str() == agent)
            .unwrap();
        plan(&definition, &self.shell(), &self.root.join("project")).unwrap()
    }
}

fn var<'a>(plan: &'a LaunchPlan, name: &str) -> Option<&'a str> {
    plan.env
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

#[test]
fn claude_code_gets_its_servers_through_mcp_config_for_this_session_only() {
    let m = machine();
    let before = m.plan("claude-code");
    let plan = attach_mcp(
        before.clone(),
        BOTH,
        &[
            stdio("github"),
            remote("docs", "https://mcp.example.com/mcp"),
        ],
    )
    .unwrap();
    assert_eq!(plan.extra_args[0], "--mcp-config");
    let config: serde_json::Value = serde_json::from_str(&plan.extra_args[1]).unwrap();
    assert_eq!(
        config,
        serde_json::json!({ "mcpServers": {
            "x8ai-github": {
                "type": "stdio",
                "command": "/Applications/x8ai Workspace.app/Contents/MacOS/x8ai-desktop",
                "args": ["--mcp-bridge", "/Users/me/.x8ai/mcp/1-abcdef01/github.sock"]
            },
            "x8ai-docs": { "type": "http", "url": "https://mcp.example.com/mcp" }
        }})
    );
    // Nothing else changes: the environment, the approved arguments.
    assert_eq!(plan.env, before.env);
    assert_eq!(plan.args, before.args);
    assert_eq!(plan.mcp, [id("github"), id("docs")]);
    // Not --strict-mcp-config: the user's own servers stay theirs.
    assert!(!plan.extra_args.iter().any(|a| a == "--strict-mcp-config"));
}

#[test]
fn claude_code_gets_the_model_and_the_servers_together() {
    let m = machine();
    let provider = x8ai_providers::builtin()
        .into_iter()
        .find(|p| p.id.as_str() == "anthropic")
        .unwrap();
    let key = SecretValue::new("sk-x8ai-test-invalid-0000").unwrap();
    let configured = configure(
        m.plan("claude-code"),
        &provider,
        "claude-sonnet-5",
        Some(&key),
    )
    .unwrap();
    let plan = attach_mcp(configured, BOTH, &[stdio("github")]).unwrap();
    assert_eq!(&plan.extra_args[..2], ["--model", "claude-sonnet-5"]);
    assert_eq!(plan.extra_args[2], "--mcp-config");
    // The MCP configuration holds no key, and the agent's key is not the servers'.
    assert!(!plan.extra_args[3].contains("sk-"));
}

#[test]
fn opencode_gets_its_servers_in_the_inline_configuration_it_already_has() {
    let m = machine();
    // No configuration yet: a new one.
    let plan = attach_mcp(
        m.plan("opencode"),
        BOTH,
        &[
            stdio("github"),
            remote("docs", "https://mcp.example.com/mcp"),
        ],
    )
    .unwrap();
    let config: serde_json::Value =
        serde_json::from_str(var(&plan, "OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
    assert_eq!(
        config["mcp"]["x8ai-github"],
        serde_json::json!({
            "type": "local",
            "command": [
                "/Applications/x8ai Workspace.app/Contents/MacOS/x8ai-desktop",
                "--mcp-bridge",
                "/Users/me/.x8ai/mcp/1-abcdef01/github.sock"
            ],
            "enabled": true
        })
    );
    assert_eq!(
        config["mcp"]["x8ai-docs"],
        serde_json::json!({ "type": "remote", "url": "https://mcp.example.com/mcp", "enabled": true })
    );
    assert!(plan.extra_args.is_empty());

    // With the app's provider configuration: merged into it.
    let provider = x8ai_providers::builtin()
        .into_iter()
        .find(|p| p.id.as_str() == "openrouter")
        .unwrap();
    let key = SecretValue::new("sk-x8ai-test-invalid-0000").unwrap();
    let configured = configure(
        m.plan("opencode"),
        &provider,
        "anthropic/claude-sonnet-5",
        Some(&key),
    )
    .unwrap();
    let plan = attach_mcp(configured, BOTH, &[stdio("github")]).unwrap();
    let config: serde_json::Value =
        serde_json::from_str(var(&plan, "OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
    assert_eq!(config["model"], "openrouter/anthropic/claude-sonnet-5");
    assert!(config["mcp"]["x8ai-github"].is_object());
    assert_eq!(
        var(&plan, "OPENROUTER_API_KEY"),
        Some("sk-x8ai-test-invalid-0000")
    );

    // With the shell's own inline configuration: merged too, the user's servers kept.
    let mut own = m.plan("opencode");
    own.env.push((
        "OPENCODE_CONFIG_CONTENT".into(),
        r#"{"model":"anthropic/claude-haiku-4-5","mcp":{"mine":{"type":"remote","url":"https://mine.example/mcp"}}}"#.into(),
    ));
    let plan = attach_mcp(own, BOTH, &[stdio("github")]).unwrap();
    let config: serde_json::Value =
        serde_json::from_str(var(&plan, "OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
    assert_eq!(config["model"], "anthropic/claude-haiku-4-5");
    assert!(config["mcp"]["mine"].is_object() && config["mcp"]["x8ai-github"].is_object());
    assert_eq!(
        plan.env
            .iter()
            .filter(|(n, _)| n == "OPENCODE_CONFIG_CONTENT")
            .count(),
        1
    );
}

#[test]
fn an_agent_without_mcp_support_gets_nothing_and_says_why() {
    let m = machine();
    let codex: AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "codex",
        "name": "Codex",
        "launch": { "program": "codex" },
        "capabilities": { "modelApis": ["openAiResponses"], "mcpTransports": ["stdio", "streamableHttp"] }
    }))
    .unwrap();
    let plan = plan(&codex, &m.shell(), &m.root.join("project")).unwrap();
    let reason = mcp_support("codex", BOTH).unwrap_err();
    assert!(reason.contains("no adapter"), "{reason}");
    let error = attach_mcp(plan.clone(), BOTH, &[stdio("github")]).unwrap_err();
    assert!(
        matches!(error, ConfigureError::McpUnsupported { .. }),
        "{error}"
    );
    // No servers: nothing to refuse, nothing changes.
    assert_eq!(attach_mcp(plan.clone(), BOTH, &[]).unwrap(), plan);

    // A transport the agent's definition does not declare.
    let error = attach_mcp(
        m.plan("claude-code"),
        &[McpTransportKind::Stdio],
        &[remote("docs", "https://x.example/mcp")],
    )
    .unwrap_err();
    assert!(error.to_string().contains("transport"), "{error}");
    assert!(mcp_support("claude-code", &[]).is_err(), "declares none");
    assert!(mcp_support("claude-code", BOTH).is_ok() && mcp_support("opencode", BOTH).is_ok());
}

#[derive(Default)]
struct Recorder {
    exit: Mutex<Option<TerminalExit>>,
    changed: Condvar,
}

impl SessionEvents for Recorder {
    fn output(&self, _: Vec<u8>) {}
    fn error(&self, _: String) {}
    fn exited(&self, exit: TerminalExit) {
        *self.exit.lock().unwrap() = Some(exit);
        self.changed.notify_all();
    }
}

impl Recorder {
    fn wait(&self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut exit = self.exit.lock().unwrap();
        while exit.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "the agent did not finish");
            exit = self.changed.wait_timeout(exit, left).unwrap().0;
        }
    }
}

/// Every file under `dir`, relative.
fn files(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push(path.strip_prefix(dir).unwrap().display().to_string());
            }
        }
    }
    out.sort();
    out
}

#[test]
fn a_session_keeps_its_servers_and_nothing_global_is_written() {
    let m = machine();
    let (mut trust, _) = TrustStore::load(m.root.join("data/trusted.json"));
    let (mut approvals, _) = ApprovalStore::load(m.root.join("data/approvals.json"));
    let workspace = m.root.join("project");
    trust.set(&workspace, true).unwrap();
    let plan = attach_mcp(m.plan("claude-code"), BOTH, &[stdio("github"), stdio("db")]).unwrap();
    approvals.approve(&plan.approval()).unwrap();
    let runtime = AgentRuntime::default();
    let sessions = Sessions::default();
    let session = runtime.create(&plan, workspace.clone(), None).unwrap();
    assert_eq!(runtime.get(session).unwrap().mcp, [id("github"), id("db")]);

    let recorder = Arc::new(Recorder::default());
    runtime
        .run(
            &sessions,
            session,
            authorize(&plan, &trust, &approvals).unwrap(),
            TerminalSize { cols: 80, rows: 24 },
            recorder.clone(),
        )
        .unwrap();
    recorder.wait();
    let args = fs::read_to_string(m.root.join("home/agent-args")).unwrap();
    assert!(
        args.contains("--mcp-config") && args.contains("x8ai-github"),
        "{args}"
    );
    // Nothing global: the agent's home has only what the fake agent itself wrote.
    assert_eq!(
        files(&m.root.join("home")),
        ["agent-args", "agent-opencode-config"]
    );

    // A later run may leave a server out (disabled since), never add one.
    let fewer = attach_mcp(m.plan("claude-code"), BOTH, &[stdio("db")]).unwrap();
    let recorder = Arc::new(Recorder::default());
    runtime
        .run(
            &sessions,
            session,
            authorize(&fewer, &trust, &approvals).unwrap(),
            TerminalSize { cols: 80, rows: 24 },
            recorder.clone(),
        )
        .unwrap();
    recorder.wait();
    let more = attach_mcp(
        m.plan("claude-code"),
        BOTH,
        &[stdio("github"), stdio("db"), stdio("new")],
    )
    .unwrap();
    let result = runtime.run(
        &sessions,
        session,
        authorize(&more, &trust, &approvals).unwrap(),
        TerminalSize { cols: 80, rows: 24 },
        Arc::new(Recorder::default()),
    );
    assert!(matches!(result, Err(RunError::Mismatch)));

    // Two sessions, each with its own servers.
    let other = attach_mcp(m.plan("claude-code"), BOTH, &[stdio("docs")]).unwrap();
    let second = runtime.create(&other, workspace, None).unwrap();
    assert_eq!(runtime.get(second).unwrap().mcp, [id("docs")]);
    assert_eq!(runtime.get(session).unwrap().mcp, [id("github"), id("db")]);
}

#[test]
fn a_worktree_remembers_its_sessions_servers_by_id_only() {
    let m = machine();
    let repo_dir = m.root.join("project");
    fs::write(repo_dir.join("README.md"), "# test\n").unwrap();
    let git = |args: &[&str]| {
        let status = std::process::Command::new("git")
            .args([
                "-c",
                "user.name=T",
                "-c",
                "user.email=t@example.com",
                "-c",
                "init.defaultBranch=main",
            ])
            .args(args)
            .current_dir(&repo_dir)
            .output()
            .unwrap();
        assert!(status.status.success());
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "start"]);
    let git_cli = x8ai_git::Git::new(PathBuf::from("/usr/bin/git"), &m.shell());
    let repo = git_cli.repository(&repo_dir).unwrap().unwrap();
    let isolation = x8ai_agents::Isolation::new(m.root.join("home/.x8ai/worktrees"));
    let worktree = isolation
        .create(
            &git_cli,
            &repo,
            &id("claude-code"),
            None,
            &[id("github"), id("db")],
            &[],
        )
        .unwrap();
    let found = isolation.find(&git_cli, &repo).unwrap();
    assert_eq!(found[0].mcp, [id("github"), id("db")]);
    let metadata = worktree.path.with_extension("json");
    let text = fs::read_to_string(&metadata).unwrap();
    assert!(text.contains(r#""mcp""#) && !text.contains("command") && !text.contains("TOKEN"));

    // An id the app would never make: the entry is not used.
    fs::write(&metadata, text.replace("\"db\"", "\"../../etc\"")).unwrap();
    assert!(isolation.find(&git_cli, &repo).unwrap().is_empty());
    isolation.remove(&git_cli, &repo, &worktree, true).unwrap();
}
