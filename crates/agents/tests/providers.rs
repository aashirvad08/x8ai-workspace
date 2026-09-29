//! Pointing agents at providers: the adapters, the environment precedence rule,
//! what an approval covers, and where a credential goes (docs/models.md,
//! ADR 0014-0016). The credential is a made-up, clearly invalid key; a small
//! script named `claude` stands in for Claude Code and reports what it received,
//! the key only as a hash.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use x8ai_agents::adapter::{ConfigureError, configure, shell_variables, support};
use x8ai_agents::{AgentRuntime, Isolation, LaunchPlan, RunError, authorize, builtin, plan};
use x8ai_core::agent::{AgentDefinition, SessionConfiguration};
use x8ai_core::model::{CredentialState, ModelProviderDefinition, ModelSelection};
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_git::Git;
use x8ai_pty::{Environment, Program, SessionEvents, Sessions};
use x8ai_secrets::SecretValue;
use x8ai_workspace::{ApprovalStore, TrustStore};

/// Not a key: never valid anywhere.
const TEST_KEY: &str = "sk-x8ai-test-0000-invalid-not-a-real-key";
const SHELL_KEY: &str = "sk-shell-test-1111-invalid";

const SIZE: TerminalSize = TerminalSize {
    cols: 100,
    rows: 30,
};

const FAKE_CLAUDE: &str = r#"#!/bin/sh
hash() { printf '%s' "$1" | shasum -a 256 | cut -c1-16; }
echo "args=$*"
echo "api-key=${ANTHROPIC_API_KEY+$(hash "$ANTHROPIC_API_KEY")}"
echo "auth-token=${ANTHROPIC_AUTH_TOKEN-unset}"
echo "base-url=${ANTHROPIC_BASE_URL-unset}"
echo "managed=${CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST-unset}"
echo "bedrock=${CLAUDE_CODE_USE_BEDROCK-unset}"
echo "editor=${EDITOR-unset}"
echo "done"
"#;

fn provider(id: &str) -> ModelProviderDefinition {
    x8ai_providers::builtin()
        .into_iter()
        .find(|p| p.id.as_str() == id)
        .unwrap()
}

fn agent(id: &str) -> AgentDefinition {
    builtin().into_iter().find(|a| a.id.as_str() == id).unwrap()
}

fn key() -> SecretValue {
    SecretValue::new(TEST_KEY).unwrap()
}

fn hash(value: &str) -> String {
    let output = Command::new("/bin/sh")
        .args([
            "-c",
            "printf '%s' \"$1\" | shasum -a 256 | cut -c1-16",
            "sh",
            value,
        ])
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn var<'a>(plan: &'a LaunchPlan, name: &str) -> Option<&'a str> {
    plan.env
        .iter()
        .find(|(n, _)| n == name)
        .map(|(_, v)| v.as_str())
}

/// A directory with fake `claude` and `opencode` on its "PATH", and a login
/// environment that already configures a provider of its own.
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
    for program in ["claude", "opencode"] {
        fs::write(bin.join(program), FAKE_CLAUDE).unwrap();
        fs::set_permissions(bin.join(program), fs::Permissions::from_mode(0o755)).unwrap();
    }
    fs::create_dir_all(root.join("project")).unwrap();
    Machine {
        _temp: temp,
        root,
        bin,
    }
}

impl Machine {
    fn shell(&self) -> Vec<(String, String)> {
        [
            ("PATH", format!("{}:/usr/bin:/bin", self.bin.display())),
            ("HOME", self.root.display().to_string()),
            ("EDITOR", "vim".into()),
            ("ANTHROPIC_API_KEY", SHELL_KEY.into()),
            ("ANTHROPIC_BASE_URL", "https://proxy.example".into()),
            ("ANTHROPIC_MODEL", "claude-from-shell".into()),
            ("CLAUDE_CODE_USE_BEDROCK", "1".into()),
            ("OPENAI_API_KEY", SHELL_KEY.into()),
        ]
        .into_iter()
        .map(|(n, v)| (n.to_owned(), v))
        .collect()
    }

    fn workspace(&self) -> PathBuf {
        self.root.join("project")
    }

    fn plan(&self, agent_id: &str) -> LaunchPlan {
        plan(&agent(agent_id), &self.shell(), &self.workspace()).unwrap()
    }

    fn configured(&self, agent_id: &str, provider_id: &str, model: &str) -> LaunchPlan {
        configure(
            self.plan(agent_id),
            &provider(provider_id),
            model,
            Some(&key()),
        )
        .unwrap()
    }
}

#[test]
fn without_a_provider_the_agent_keeps_the_shells_configuration_untouched() {
    let m = machine();
    let plan = m.plan("claude-code");
    assert_eq!(plan.env, m.shell());
    assert!(plan.extra_args.is_empty());
    assert_eq!(plan.provider, None);
    assert_eq!(plan.model, None);
    assert_eq!(plan.approval().provider, None);
    // The user is told which shell variables decide the agent's provider.
    assert_eq!(
        plan.configuration,
        SessionConfiguration::Agent {
            shell_variables: vec![
                "ANTHROPIC_API_KEY".into(),
                "ANTHROPIC_BASE_URL".into(),
                "ANTHROPIC_MODEL".into(),
                "CLAUDE_CODE_USE_BEDROCK".into(),
            ]
        }
    );
}

#[test]
fn claude_code_with_anthropic_uses_the_apps_key_and_nothing_from_the_shell() {
    let m = machine();
    let plan = m.configured("claude-code", "anthropic", "claude-sonnet-5");
    assert_eq!(var(&plan, "ANTHROPIC_API_KEY"), Some(TEST_KEY));
    assert_eq!(
        var(&plan, "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"),
        Some("1")
    );
    // The shell's endpoint, model and provider switch are gone, not mixed in.
    for gone in [
        "ANTHROPIC_BASE_URL",
        "ANTHROPIC_MODEL",
        "CLAUDE_CODE_USE_BEDROCK",
        "ANTHROPIC_AUTH_TOKEN",
    ] {
        assert_eq!(var(&plan, gone), None, "{gone}");
    }
    // Everything else of the shell is still there.
    assert_eq!(var(&plan, "EDITOR"), Some("vim"));
    assert_eq!(
        var(&plan, "OPENAI_API_KEY"),
        Some(SHELL_KEY),
        "not Claude Code's"
    );
    assert!(
        var(&plan, "PATH")
            .unwrap()
            .starts_with(m.bin.to_str().unwrap())
    );
    assert_eq!(plan.extra_args, ["--model", "claude-sonnet-5"]);
    let route = plan.provider.clone().unwrap();
    assert_eq!(
        (route.provider.as_str(), route.endpoint.as_str()),
        ("anthropic", "https://api.anthropic.com")
    );
    assert_eq!(
        plan.configuration,
        SessionConfiguration::App {
            provider: route.provider,
            provider_name: "Anthropic".into(),
            model: "claude-sonnet-5".into(),
            endpoint: "https://api.anthropic.com".into(),
            credential: CredentialState::InKeychain,
            overridden_shell_variables: vec![
                "ANTHROPIC_API_KEY".into(),
                "ANTHROPIC_BASE_URL".into(),
                "ANTHROPIC_MODEL".into(),
                "CLAUDE_CODE_USE_BEDROCK".into(),
            ],
        }
    );
    // Exactly one of each variable.
    let mut names: Vec<&str> = plan.env.iter().map(|(n, _)| n.as_str()).collect();
    names.sort();
    let before = names.len();
    names.dedup();
    assert_eq!(names.len(), before);
}

#[test]
fn claude_code_reaches_openrouter_and_ollama_through_their_anthropic_endpoints() {
    let m = machine();
    let plan = m.configured("claude-code", "openrouter", "anthropic/claude-sonnet-5");
    assert_eq!(
        var(&plan, "ANTHROPIC_BASE_URL"),
        Some("https://openrouter.ai/api")
    );
    assert_eq!(var(&plan, "ANTHROPIC_AUTH_TOKEN"), Some(TEST_KEY));
    assert_eq!(
        var(&plan, "ANTHROPIC_API_KEY"),
        Some(""),
        "explicitly empty"
    );
    for alias in ["OPUS", "SONNET", "HAIKU", "FABLE"] {
        assert_eq!(
            var(&plan, &format!("ANTHROPIC_DEFAULT_{alias}_MODEL")),
            Some("anthropic/claude-sonnet-5")
        );
    }
    assert_eq!(plan.extra_args, ["--model", "anthropic/claude-sonnet-5"]);

    // Ollama needs no key; its documented placeholder token is used.
    let plan = configure(
        m.plan("claude-code"),
        &provider("ollama"),
        "qwen3-coder:30b",
        None,
    )
    .unwrap();
    assert_eq!(
        var(&plan, "ANTHROPIC_BASE_URL"),
        Some("http://localhost:11434")
    );
    assert_eq!(var(&plan, "ANTHROPIC_AUTH_TOKEN"), Some("ollama"));
    assert_eq!(var(&plan, "ANTHROPIC_API_KEY"), Some(""));
    assert!(matches!(
        plan.configuration,
        SessionConfiguration::App {
            credential: CredentialState::NotNeeded,
            ..
        }
    ));
}

#[test]
fn support_is_decided_by_the_agents_adapter_and_says_why_not() {
    let expected = [
        ("claude-code", "anthropic", true),
        ("claude-code", "openai", false),
        ("claude-code", "google", false),
        ("claude-code", "openrouter", true),
        ("claude-code", "ollama", true),
        ("opencode", "anthropic", true),
        ("opencode", "openai", true),
        ("opencode", "google", true),
        ("opencode", "openrouter", true),
        ("opencode", "ollama", true),
    ];
    for (agent, provider_id, supported) in expected {
        let result = support(agent, &provider(provider_id));
        assert_eq!(
            result.is_ok(),
            supported,
            "{agent} with {provider_id}: {result:?}"
        );
    }
    let reason = support("claude-code", &provider("openai")).unwrap_err();
    assert!(reason.contains("Anthropic Messages API"), "{reason}");

    let m = machine();
    let error = configure(
        m.plan("claude-code"),
        &provider("google"),
        "gemini-pro",
        Some(&key()),
    )
    .unwrap_err();
    assert!(
        matches!(error, ConfigureError::Unsupported { .. }),
        "{error}"
    );
}

#[test]
fn an_agent_without_an_adapter_only_uses_its_own_configuration() {
    let m = machine();
    let other: AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "some-agent",
        "name": "Some Agent",
        "launch": { "program": "claude" },
        "capabilities": { "modelApis": ["anthropicMessages"] }
    }))
    .unwrap();
    let plan = plan(&other, &m.shell(), &m.workspace()).unwrap();
    assert!(support("some-agent", &provider("anthropic")).is_err());
    assert!(shell_variables("some-agent", &m.shell()).is_empty());
    let error = configure(
        plan,
        &provider("anthropic"),
        "claude-sonnet-5",
        Some(&key()),
    )
    .unwrap_err();
    assert!(matches!(error, ConfigureError::NoAdapter(_)));
}

#[test]
fn a_provider_that_needs_a_key_refuses_to_start_without_one() {
    let m = machine();
    for provider_id in ["anthropic", "openrouter"] {
        let error = configure(
            m.plan("claude-code"),
            &provider(provider_id),
            "some-model",
            None,
        )
        .unwrap_err();
        assert!(
            matches!(error, ConfigureError::MissingCredential(_)),
            "{error}"
        );
    }
}

#[test]
fn a_model_id_can_never_become_a_command_line_option() {
    let m = machine();
    for bad in ["--dangerously-skip-permissions", "-p", "a b", "", "$(id)"] {
        let error = configure(
            m.plan("claude-code"),
            &provider("anthropic"),
            bad,
            Some(&key()),
        )
        .unwrap_err();
        assert!(
            matches!(error, ConfigureError::InvalidModel(_)),
            "{bad}: {error}"
        );
    }
}

#[test]
fn opencode_gets_the_model_and_a_pinned_endpoint_in_its_inline_configuration() {
    let m = machine();
    let plan = m.configured("opencode", "openrouter", "anthropic/claude-sonnet-5");
    assert_eq!(var(&plan, "OPENROUTER_API_KEY"), Some(TEST_KEY));
    // Keys the shell had for other providers OpenCode reads are replaced too.
    assert_eq!(var(&plan, "OPENAI_API_KEY"), None);
    assert_eq!(var(&plan, "ANTHROPIC_API_KEY"), None);
    assert!(plan.extra_args.is_empty(), "no undocumented flags");
    let config: serde_json::Value =
        serde_json::from_str(var(&plan, "OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
    assert_eq!(config["model"], "openrouter/anthropic/claude-sonnet-5");
    assert_eq!(
        config["provider"]["openrouter"]["options"]["baseURL"],
        "https://openrouter.ai/api/v1"
    );
    // The key is not in the configuration, only in its variable.
    assert!(!config.to_string().contains(TEST_KEY));

    let plan = m.configured("opencode", "anthropic", "claude-sonnet-5");
    assert_eq!(var(&plan, "ANTHROPIC_API_KEY"), Some(TEST_KEY));
    assert_eq!(
        plan.provider.unwrap().endpoint,
        "https://api.anthropic.com/v1"
    );

    let plan = configure(
        m.plan("opencode"),
        &provider("ollama"),
        "qwen3-coder:30b",
        None,
    )
    .unwrap();
    let config: serde_json::Value =
        serde_json::from_str(var(&plan, "OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
    assert_eq!(config["model"], "ollama/qwen3-coder:30b");
    let ollama = &config["provider"]["ollama"];
    assert_eq!(ollama["npm"], "@ai-sdk/openai-compatible");
    assert_eq!(ollama["options"]["baseURL"], "http://localhost:11434/v1");
    assert_eq!(
        ollama["models"]["qwen3-coder:30b"]["name"],
        "qwen3-coder:30b"
    );
}

#[test]
fn a_credential_never_appears_in_debug_output_or_errors() {
    let m = machine();
    let plan = m.configured("claude-code", "openrouter", "anthropic/claude-sonnet-5");
    let printed = format!("{plan:?}");
    assert!(
        !printed.contains(TEST_KEY) && !printed.contains(SHELL_KEY),
        "{printed}"
    );
    assert!(
        printed.contains("ANTHROPIC_AUTH_TOKEN"),
        "names are shown: {printed}"
    );
    let error = configure(
        m.plan("claude-code"),
        &provider("google"),
        "x",
        Some(&key()),
    )
    .unwrap_err();
    assert!(!format!("{error} {error:?}").contains(TEST_KEY));
}

#[test]
fn a_new_provider_or_endpoint_needs_approval_and_a_new_model_does_not() {
    let m = machine();
    let (mut trust, _) = TrustStore::load(m.root.join("data/trusted.json"));
    let (mut approvals, _) = ApprovalStore::load(m.root.join("data/approvals.json"));
    trust.set(&m.workspace(), true).unwrap();

    let sonnet = m.configured("claude-code", "anthropic", "claude-sonnet-5");
    assert!(authorize(&sonnet, &trust, &approvals).is_err());
    // Approving the agent's own configuration does not approve a provider.
    approvals
        .approve(&m.plan("claude-code").approval())
        .unwrap();
    assert!(authorize(&sonnet, &trust, &approvals).is_err());
    approvals.approve(&sonnet.approval()).unwrap();
    assert!(authorize(&sonnet, &trust, &approvals).is_ok());

    // Another model from the same provider: same destination, same key.
    let haiku = m.configured("claude-code", "anthropic", "claude-haiku-4-5-20251001");
    assert!(authorize(&haiku, &trust, &approvals).is_ok());
    // Another provider: the code and a key go somewhere else.
    let openrouter = m.configured("claude-code", "openrouter", "anthropic/claude-sonnet-5");
    assert!(authorize(&openrouter, &trust, &approvals).is_err());
    // The same provider at another endpoint.
    let mut moved = provider("anthropic");
    moved.endpoints[0].base_url = "https://anthropic-proxy.example".into();
    let moved = configure(
        m.plan("claude-code"),
        &moved,
        "claude-sonnet-5",
        Some(&key()),
    )
    .unwrap();
    assert!(authorize(&moved, &trust, &approvals).is_err());
    // A new credential for the approved provider is not a new destination.
    let rotated = configure(
        m.plan("claude-code"),
        &provider("anthropic"),
        "claude-sonnet-5",
        Some(&SecretValue::new("sk-x8ai-test-2222-rotated").unwrap()),
    )
    .unwrap();
    assert!(authorize(&rotated, &trust, &approvals).is_ok());
}

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
    fn wait_until_done(&self) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut state = self.state.lock().unwrap();
        while state.1.is_none() {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "the agent did not finish");
            state = self.changed.wait_timeout(state, left).unwrap().0;
        }
        String::from_utf8_lossy(&state.0).replace('\r', "")
    }
}

fn line<'a>(output: &'a str, name: &str) -> &'a str {
    output
        .lines()
        .find_map(|l| l.strip_prefix(&format!("{name}=")))
        .unwrap_or_else(|| panic!("no {name} in {output}"))
}

/// Every file under `dir` whose bytes contain `needle`.
fn files_containing(dir: &Path, needle: &str) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).unwrap().flatten() {
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

fn git(dir: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .output()
        .unwrap();
    assert!(status.status.success(), "git {args:?}");
}

#[test]
fn the_key_reaches_the_agent_and_is_written_nowhere() {
    let m = machine();
    let repo_dir = m.workspace();
    fs::write(repo_dir.join("README.md"), "# test\n").unwrap();
    git(&repo_dir, &["init", "-q"]);
    git(&repo_dir, &["add", "."]);
    git(&repo_dir, &["commit", "-qm", "start"]);
    let env = m.shell();
    let git_cli = Git::new(PathBuf::from("/usr/bin/git"), &env);
    let repo = git_cli.repository(&repo_dir).unwrap().unwrap();
    let isolation = Isolation::new(m.root.join("home/.x8ai/worktrees"));
    let (mut trust, _) = TrustStore::load(m.root.join("data/trusted.json"));
    let (mut approvals, _) = ApprovalStore::load(m.root.join("data/approvals.json"));
    let (mut settings, _) = x8ai_providers::Settings::load(m.root.join("data/providers.json"));
    settings
        .add_model("openrouter", "anthropic/claude-sonnet-5")
        .unwrap();
    trust.set(&repo_dir, true).unwrap();

    let plan = m.configured("claude-code", "openrouter", "anthropic/claude-sonnet-5");
    approvals.approve(&plan.approval()).unwrap();
    let selection = plan.model.clone().unwrap();
    let worktree = isolation
        .create(&git_cli, &repo, &plan.agent, Some(&selection))
        .unwrap();
    let runtime = AgentRuntime::default();
    let sessions = Sessions::default();
    let id = runtime
        .create(&plan, worktree.path.clone(), Some(worktree.clone()))
        .unwrap();
    let recorder = Arc::new(Recorder::default());
    runtime
        .run(
            &sessions,
            id,
            authorize(&plan, &trust, &approvals).unwrap(),
            SIZE,
            recorder.clone(),
        )
        .unwrap();
    let output = recorder.wait_until_done();

    // What the agent received.
    assert_eq!(line(&output, "args"), "--model anthropic/claude-sonnet-5");
    assert_eq!(line(&output, "api-key"), hash(""), "set, and empty");
    assert_eq!(line(&output, "auth-token"), TEST_KEY);
    assert_eq!(line(&output, "base-url"), "https://openrouter.ai/api");
    assert_eq!(line(&output, "managed"), "1");
    assert_eq!(line(&output, "bedrock"), "unset");
    assert_eq!(line(&output, "editor"), "vim");
    let session = runtime.get(id).unwrap();
    assert_eq!(session.model.as_ref(), Some(&selection));
    assert!(matches!(
        session.configuration,
        SessionConfiguration::App { .. }
    ));

    // Nothing the app wrote holds the key: not the project, not the worktree,
    // not the worktree's metadata, not the approval, trust or provider stores.
    assert_eq!(files_containing(&m.root, TEST_KEY), Vec::<PathBuf>::new());
    assert!(!files_containing(&m.root.join("home"), "anthropic/claude-sonnet-5").is_empty());

    // The key is in the agent's environment only: not the app's own, and so not
    // in a shell's, which starts from the app's environment as terminals do.
    assert!(!std::env::vars().any(|(_, v)| v.contains(TEST_KEY)));
    let shell = Arc::new(Recorder::default());
    sessions
        .spawn(
            &Program::Exec {
                program: PathBuf::from("/usr/bin/env"),
                args: Vec::new(),
                cwd: None,
                env: Environment::Inherit,
            },
            SIZE,
            shell.clone(),
        )
        .unwrap();
    let shell_env = shell.wait_until_done();
    assert!(shell_env.contains("PATH="), "{shell_env}");
    assert!(!shell_env.contains(TEST_KEY));

    // After a restart the session is found with its model, and runs only with it.
    let found = isolation.find(&git_cli, &repo).unwrap();
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].model.as_ref(), Some(&selection));
    let restarted = AgentRuntime::default();
    let adopted = restarted.adopt(
        "Claude Code",
        &repo_dir,
        found[0].path.clone(),
        found[0].clone(),
        plan.configuration.clone(),
    );
    let other_model = m.configured("claude-code", "openrouter", "openai/gpt-5");
    let result = restarted.run(
        &sessions,
        adopted,
        authorize(&other_model, &trust, &approvals).unwrap(),
        SIZE,
        Arc::new(Recorder::default()),
    );
    assert!(matches!(result, Err(RunError::Mismatch)));
    assert!(
        restarted
            .create(
                &m.plan("claude-code"),
                found[0].path.clone(),
                Some(found[0].clone())
            )
            .is_err(),
        "a worktree made for a model is not reused without it"
    );

    isolation.remove(&git_cli, &repo, &worktree, true).unwrap();
}

#[test]
fn a_tampered_model_in_worktree_metadata_is_not_used() {
    let m = machine();
    let repo_dir = m.workspace();
    fs::write(repo_dir.join("README.md"), "# test\n").unwrap();
    git(&repo_dir, &["init", "-q"]);
    git(&repo_dir, &["add", "."]);
    git(&repo_dir, &["commit", "-qm", "start"]);
    let git_cli = Git::new(PathBuf::from("/usr/bin/git"), &m.shell());
    let repo = git_cli.repository(&repo_dir).unwrap().unwrap();
    let isolation = Isolation::new(m.root.join("home/.x8ai/worktrees"));
    let selection = ModelSelection {
        provider: x8ai_core::id::IntegrationId::new("anthropic").unwrap(),
        model: "claude-sonnet-5".into(),
    };
    let agent_id = agent("claude-code").id;
    let worktree = isolation
        .create(&git_cli, &repo, &agent_id, Some(&selection))
        .unwrap();
    let metadata = worktree.path.with_extension("json");
    let text = fs::read_to_string(&metadata).unwrap();
    fs::write(
        &metadata,
        text.replace("claude-sonnet-5", "--dangerously-skip-permissions"),
    )
    .unwrap();
    assert!(isolation.find(&git_cli, &repo).unwrap().is_empty());
    let bad = ModelSelection {
        model: "-p".into(),
        ..selection
    };
    assert!(
        isolation
            .create(&git_cli, &repo, &agent_id, Some(&bad))
            .is_err()
    );
    isolation.remove(&git_cli, &repo, &worktree, true).unwrap();
}
