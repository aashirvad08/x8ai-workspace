//! Giving agents skills for one session through their adapters, and keeping
//! exactly the skills a session started with.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use x8ai_agents::adapter::{
    AgentSkill, ConfigureError, MAX_SKILLS_TEXT, attach_mcp, attach_skills, skills_support,
};
use x8ai_agents::{AgentRuntime, LaunchPlan, RunError, authorize, builtin, plan};
use x8ai_core::agent::AgentAvailability;
use x8ai_core::id::IntegrationId;
use x8ai_core::skill::SkillRef;
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_pty::{SessionEvents, Sessions};
use x8ai_workspace::{ApprovalStore, TrustStore};

const FAKE_AGENT: &str = r#"#!/bin/sh
for arg in "$@"; do printf '%s\n---\n' "$arg"; done > "$HOME/agent-args"
echo done
"#;

fn skill(id: &str, name: &str, instructions: &str) -> AgentSkill {
    AgentSkill {
        reference: SkillRef {
            id: IntegrationId::new(id).unwrap(),
            version: 1,
            fingerprint: "0123456789abcdef".into(),
        },
        name: name.into(),
        instructions: instructions.into(),
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
    for dir in [&bin, &root.join("home"), &root.join("project")] {
        fs::create_dir_all(dir).unwrap();
    }
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
    fn env(&self) -> Vec<(String, String)> {
        vec![
            (
                "PATH".into(),
                format!("{}:/usr/bin:/bin", self.bin.display()),
            ),
            ("HOME".into(), self.root.join("home").display().to_string()),
        ]
    }

    fn plan(&self, agent: &str) -> LaunchPlan {
        let definition = builtin()
            .into_iter()
            .find(|a| a.id.as_str() == agent)
            .unwrap();
        plan(&definition, &self.env(), &self.root.join("project")).unwrap()
    }
}

#[test]
fn claude_code_gets_the_sessions_skills_through_append_system_prompt() {
    let m = machine();
    let before = m.plan("claude-code");
    let plan = attach_skills(
        before.clone(),
        &[
            skill("python-debugging", "Python debugging", "Reproduce first."),
            skill("hep-analysis", "HEP analysis", "Use ROOT conventions."),
        ],
    )
    .unwrap();
    assert_eq!(plan.extra_args[0], "--append-system-prompt");
    let text = &plan.extra_args[1];
    assert!(
        text.contains("## Skill: Python debugging\n\nReproduce first."),
        "{text}"
    );
    assert!(text.contains("## Skill: HEP analysis\n\nUse ROOT conventions."));
    assert_eq!(
        plan.skills
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["python-debugging", "hep-analysis"]
    );
    // Nothing else: not the environment, not the approved arguments.
    assert_eq!(plan.env, before.env);
    assert_eq!(plan.args, before.args);
    // With MCP servers too, each keeps its own arguments.
    let both = attach_mcp(plan, &[x8ai_core::mcp::McpTransportKind::Stdio], &[]).unwrap();
    assert_eq!(both.extra_args.len(), 2);
}

#[test]
fn agents_that_cannot_take_skills_are_refused_with_the_reason() {
    let m = machine();
    for agent in ["opencode", "codex"] {
        let reason = skills_support(agent).unwrap_err();
        let error =
            attach_skills(m.plan(agent), &[skill("tests-first", "Tests first", "x")]).unwrap_err();
        assert!(
            matches!(error, ConfigureError::SkillsUnsupported { .. }),
            "{agent}: {error}"
        );
        assert!(!reason.is_empty());
        // No skills: nothing to refuse, nothing changes.
        assert_eq!(attach_skills(m.plan(agent), &[]).unwrap(), m.plan(agent));
    }
    assert!(skills_support("claude-code").is_ok());
    let long = "x".repeat(MAX_SKILLS_TEXT);
    assert!(matches!(
        attach_skills(m.plan("claude-code"), &[skill("big", "Big", &long)]),
        Err(ConfigureError::SkillsTooLong)
    ));
}

#[test]
fn codex_is_found_by_the_runtime_and_takes_a_model_but_no_mcp_or_skills() {
    let codex = builtin()
        .into_iter()
        .find(|a| a.id.as_str() == "codex")
        .expect("defined");
    assert_eq!(
        codex.capabilities.model_apis,
        [x8ai_core::model::ProviderApi::OpenAiResponses]
    );
    assert!(codex.capabilities.mcp_transports.is_empty());
    assert!(x8ai_agents::adapter::adapter("codex").is_some());
    assert!(skills_support("codex").is_err());
    let m = machine();
    // Installed means the runtime found its program, nothing else.
    let found = plan(&codex, &m.env(), &m.root.join("project")).unwrap();
    assert_eq!(found.program, m.bin.join("codex"));
    let missing = plan(
        &codex,
        &[("PATH".into(), "/usr/bin:/bin".into())],
        &m.root.join("project"),
    );
    assert!(matches!(
        missing,
        Err(x8ai_agents::Denied::NotInstalled { .. })
    ));
    let _ = AgentAvailability::NotInstalled {
        program: "codex".into(),
    };
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

#[test]
fn a_session_runs_only_with_the_skills_it_recorded() {
    let m = machine();
    let workspace = m.root.join("project");
    let (mut trust, _) = TrustStore::load(m.root.join("data/trusted.json"));
    let (mut approvals, _) = ApprovalStore::load(m.root.join("data/approvals.json"));
    trust.set(&workspace, true).unwrap();
    let plan = attach_skills(
        m.plan("claude-code"),
        &[skill("tests-first", "Tests first", "Write the test first.")],
    )
    .unwrap();
    approvals.approve(&plan.approval()).unwrap();
    let runtime = AgentRuntime::default();
    let sessions = Sessions::default();
    let session = runtime.create(&plan, workspace, None).unwrap();
    assert_eq!(runtime.get(session).unwrap().skills, plan.skills);

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
        args.contains("--append-system-prompt") && args.contains("Write the test first."),
        "{args}"
    );
    // Nothing global: only what the fake agent itself wrote.
    assert_eq!(fs::read_dir(m.root.join("home")).unwrap().count(), 1);

    // Another version of the skill is another launch: refused.
    let mut changed = skill("tests-first", "Tests first", "Something else.");
    changed.reference.version = 2;
    let other = attach_skills(m.plan("claude-code"), &[changed]).unwrap();
    let result = runtime.run(
        &sessions,
        session,
        authorize(&other, &trust, &approvals).unwrap(),
        TerminalSize { cols: 80, rows: 24 },
        Arc::new(Recorder::default()),
    );
    assert!(matches!(result, Err(RunError::Mismatch)));
    // So is running it without its skill.
    let without = m.plan("claude-code");
    let result = runtime.run(
        &sessions,
        session,
        authorize(&without, &trust, &approvals).unwrap(),
        TerminalSize { cols: 80, rows: 24 },
        Arc::new(Recorder::default()),
    );
    assert!(matches!(result, Err(RunError::Mismatch)));
}

#[test]
fn a_worktree_remembers_its_sessions_skills_by_reference_only() {
    let m = machine();
    let repo_dir = m.root.join("project");
    fs::write(repo_dir.join("README.md"), "# test\n").unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
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
        assert!(out.status.success());
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "start"]);
    let git_cli = x8ai_git::Git::new(PathBuf::from("/usr/bin/git"), &m.env());
    let repo = git_cli.repository(&repo_dir).unwrap().unwrap();
    let isolation = x8ai_agents::Isolation::new(m.root.join("home/.x8ai/worktrees"));
    let recorded = vec![skill("tests-first", "Tests first", "Write the test first.").reference];
    let worktree = isolation
        .create(
            &git_cli,
            &repo,
            &IntegrationId::new("claude-code").unwrap(),
            None,
            &[],
            &recorded,
        )
        .unwrap();
    assert_eq!(isolation.find(&git_cli, &repo).unwrap()[0].skills, recorded);
    let metadata = worktree.path.with_extension("json");
    let text = fs::read_to_string(&metadata).unwrap();
    assert!(
        text.contains("tests-first") && !text.contains("Write the test first."),
        "the text stays in the registry"
    );
    // Nothing of the skill inside the worktree itself.
    assert!(
        !fs::read_dir(&worktree.path)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().contains("skill"))
    );

    fs::write(
        &metadata,
        text.replace("0123456789abcdef", "not-a-fingerprint"),
    )
    .unwrap();
    assert!(
        isolation.find(&git_cli, &repo).unwrap().is_empty(),
        "a tampered entry is not used"
    );
    isolation.remove(&git_cli, &repo, &worktree, true).unwrap();
}
