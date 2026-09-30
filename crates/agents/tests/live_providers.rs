//! Claude Code, configured by the app for a provider, for real. Not run by default:
//!
//! ```sh
//! cargo test -p x8ai-agents --test live_providers -- --ignored --nocapture
//! ```
//!
//! The credential is clearly invalid (`sk-ant-x8ai-invalid-…`) and lives in a
//! throwaway Keychain service that is deleted at the end. Claude Code is never
//! sent a prompt, so no model request is made. What reaches it is read from its
//! process (`ps -E`); the key is compared, never printed. Everything else is in a
//! temporary directory: a disposable Git repository, the worktree root, and the
//! trust, approval and provider stores.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use x8ai_agents::adapter::{ConfigureError, configure};
use x8ai_agents::environment::{RESOLVE_TIMEOUT, resolve};
use x8ai_agents::{AgentRuntime, Denied, Isolation, LaunchPlan, authorize, builtin, plan};
use x8ai_core::agent::SessionConfiguration;
use x8ai_core::model::{LocalAvailability, ModelProviderDefinition};
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_git::Git;
use x8ai_pty::{SessionEvents, Sessions, user_shell};
use x8ai_secrets::{Keychain, SecretStore, SecretValue};
use x8ai_workspace::{ApprovalStore, TrustStore};

const INVALID_KEY: &str = "sk-ant-x8ai-invalid-test-key-0000000000";
const SIZE: TerminalSize = TerminalSize {
    cols: 120,
    rows: 36,
};

#[derive(Default)]
struct Screen {
    state: Mutex<(Vec<u8>, Option<TerminalExit>)>,
    changed: Condvar,
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

impl Screen {
    /// The text drawn so far, escape sequences removed.
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
            } else if !c.is_control() || c == '\n' {
                text.push(c);
            }
        }
        text
    }

    fn wait_for_output(&self, bytes: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut state = self.state.lock().unwrap();
        while state.0.len() < bytes {
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

/// What the kernel says the process was started with: its arguments and its
/// environment, as `NAME=value` words.
fn process_of(pid: u32) -> (String, Vec<String>) {
    let args = run(
        Path::new("/"),
        "ps",
        &["-ww", "-o", "args=", "-p", &pid.to_string()],
    );
    let with_env = run(
        Path::new("/"),
        "ps",
        &["-E", "-ww", "-o", "command=", "-p", &pid.to_string()],
    );
    let env = with_env
        .strip_prefix(&args)
        .unwrap_or(&with_env)
        .split(' ')
        .filter(|w| w.contains('='))
        .map(str::to_owned)
        .collect();
    (args, env)
}

fn value<'a>(env: &'a [String], name: &str) -> Option<&'a str> {
    env.iter()
        .find_map(|w| w.strip_prefix(name).and_then(|rest| rest.strip_prefix('=')))
}

fn gone_within(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
    {
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    true
}

fn provider(id: &str) -> ModelProviderDefinition {
    x8ai_providers::builtin()
        .into_iter()
        .find(|p| p.id.as_str() == id)
        .unwrap()
}

/// Every file under `dir` containing `needle`.
fn files_containing(dir: &Path, needle: &[u8]) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_owned()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
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
                && std::fs::read(&path).is_ok_and(|b| b.windows(needle.len()).any(|w| w == needle))
            {
                found.push(path);
            }
        }
    }
    found
}

#[test]
#[ignore = "needs Claude Code installed and the login Keychain; run by hand"]
fn claude_code_gets_the_provider_the_app_configures_and_nothing_leaks() {
    let temp = tempfile::tempdir().unwrap();
    let base = std::fs::canonicalize(temp.path()).unwrap();
    let repo_dir = base.join("disposable-project");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join("README.md"), "# disposable test project\n").unwrap();
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

    // 1-4. The credential goes into the Keychain; a new store instance (as after
    // a restart) finds it; nothing that is printed shows it.
    let service = format!("com.x8ai.workspace.test.live.{}", std::process::id());
    let keychain = Keychain::new(&service, "x8ai live test (invalid key)");
    let key = SecretValue::new(INVALID_KEY).unwrap();
    keychain.set("anthropic", &key).unwrap();
    let after_restart = Keychain::new(&service, "x8ai live test (invalid key)");
    assert!(after_restart.contains("anthropic").unwrap());
    let loaded = after_restart.get("anthropic").unwrap().unwrap();
    assert_eq!(loaded, key);
    assert_eq!(format!("{loaded:?}"), "SecretValue(<redacted>)");
    println!(
        "1-4. key saved in a throwaway Keychain service, found again by a new instance, printed only as {loaded:?}"
    );

    // The user's real login environment, plus a shell that already configures
    // a provider of its own, and an unrelated variable.
    let home = std::env::home_dir().unwrap();
    let mut login = resolve(&PathBuf::from(user_shell()), &home, RESOLVE_TIMEOUT).unwrap();
    // This test may itself run inside a Claude Code session, whose own variables
    // an app started from the Dock would not have.
    login.retain(|(n, _)| {
        !n.starts_with("ANTHROPIC_")
            && !n.starts_with("CLAUDE_CODE_")
            && n != "CLAUDECODE"
            && n != "X8AI_UNRELATED"
    });
    login.push((
        "ANTHROPIC_BASE_URL".into(),
        "https://shell-proxy.invalid".into(),
    ));
    login.push(("ANTHROPIC_MODEL".into(), "model-from-shell".into()));
    login.push(("X8AI_UNRELATED".into(), "kept".into()));

    let claude = builtin()
        .into_iter()
        .find(|a| a.id.as_str() == "claude-code")
        .unwrap();
    let own = plan(&claude, &login, &repo_dir).expect("Claude Code is installed");
    let git = Git::new(PathBuf::from("/usr/bin/git"), &login);
    let repo = git.repository(&repo_dir).unwrap().unwrap();
    let isolation = Isolation::new(base.join("home/.x8ai/worktrees"));
    let (mut trust, _) = TrustStore::load(base.join("data/trusted.json"));
    let (mut approvals, _) = ApprovalStore::load(base.join("data/approvals.json"));
    let (mut settings, _) = x8ai_providers::Settings::load(base.join("data/providers.json"));
    settings
        .add_model("anthropic", "claude-haiku-4-5-20251001")
        .unwrap();
    trust.set(&repo_dir, true).unwrap();
    approvals.approve(&own.approval()).unwrap();

    // 5. Select a model; the plan names the provider and the endpoint.
    let configured =
        |model: &str, credential: Option<&SecretValue>| -> Result<LaunchPlan, ConfigureError> {
            configure(own.clone(), &provider("anthropic"), model, credential)
        };
    let haiku = configured("claude-haiku-4-5-20251001", Some(&loaded)).unwrap();
    println!(
        "5. model chosen: {:?}, endpoint {:?}",
        haiku
            .model
            .as_ref()
            .map(|m| format!("{}/{}", m.provider, m.model)),
        haiku.provider.as_ref().map(|p| &p.endpoint)
    );

    // 10. The agent's own configuration was approved; the provider was not.
    assert!(matches!(
        authorize(&haiku, &trust, &approvals),
        Err(Denied::NotApproved { .. })
    ));
    approvals.approve(&haiku.approval()).unwrap();
    assert!(authorize(&haiku, &trust, &approvals).is_ok());
    let sonnet = configured("claude-sonnet-5", Some(&loaded)).unwrap();
    assert!(authorize(&sonnet, &trust, &approvals).is_ok());
    let openrouter = configure(
        own.clone(),
        &provider("openrouter"),
        "anthropic/claude-sonnet-5",
        Some(&loaded),
    )
    .unwrap();
    assert!(authorize(&openrouter, &trust, &approvals).is_err());
    println!("10. provider needs its own approval; another model does not; another provider does");

    // 6. A session with a worktree of its own, keeping the model.
    let worktree = isolation
        .create(&git, &repo, &claude.id, haiku.model.as_ref(), &[], &[])
        .unwrap();
    let runtime = AgentRuntime::default();
    let sessions = Sessions::default();
    let id = runtime
        .create(&haiku, worktree.path.clone(), Some(worktree.clone()))
        .unwrap();
    let screen = Arc::new(Screen::default());
    let pty = runtime
        .run(
            &sessions,
            id,
            authorize(&haiku, &trust, &approvals).unwrap(),
            SIZE,
            screen.clone(),
        )
        .unwrap();
    let pid = pty.pid().unwrap();
    let drew = screen.wait_for_output(200);
    std::thread::sleep(Duration::from_secs(3));
    println!(
        "6. Claude Code started in {} (pid {pid}), drew its interface: {drew}",
        worktree.path.display()
    );

    // 7-9. What Claude Code was started with.
    let (args, env) = process_of(pid);
    assert!(
        args.ends_with("--model claude-haiku-4-5-20251001"),
        "{args}"
    );
    assert_eq!(
        value(&env, "ANTHROPIC_API_KEY"),
        Some(INVALID_KEY),
        "the key reached it"
    );
    assert_eq!(
        value(&env, "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST"),
        Some("1")
    );
    assert_eq!(
        value(&env, "ANTHROPIC_BASE_URL"),
        None,
        "the shell's endpoint is gone"
    );
    assert_eq!(
        value(&env, "ANTHROPIC_MODEL"),
        None,
        "the shell's model is gone"
    );
    assert_eq!(value(&env, "X8AI_UNRELATED"), Some("kept"));
    assert!(value(&env, "PATH").is_some() && value(&env, "HOME").is_some());
    let names: Vec<&str> = env
        .iter()
        .filter_map(|w| w.split_once('=').map(|(n, _)| n))
        .filter(|n| n.starts_with("ANTHROPIC_") || n.starts_with("CLAUDE_CODE_"))
        .collect();
    println!("7. argv ends with: --model claude-haiku-4-5-20251001");
    println!("7. provider variables it has (values not shown): {names:?}");
    println!(
        "8. unrelated variables kept: X8AI_UNRELATED, PATH, HOME, … ({} variables)",
        env.len()
    );
    if let SessionConfiguration::App {
        overridden_shell_variables,
        ..
    } = &haiku.configuration
    {
        println!("9. replaced from the shell: {overridden_shell_variables:?}");
        assert_eq!(
            overridden_shell_variables,
            &["ANTHROPIC_BASE_URL", "ANTHROPIC_MODEL"]
        );
    }
    let text = screen.text();
    assert!(!text.contains(INVALID_KEY), "the full key is never drawn");
    let shown: Vec<String> = text
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| l.len() > 3)
        .map(|l| {
            // A fragment of the key, if Claude Code shows one, is not printed either.
            l.split(' ')
                .map(|w| if w.contains("sk-ant") { "<key>" } else { w })
                .collect::<Vec<_>>()
                .join(" ")
        })
        .take(12)
        .collect();
    println!("   Claude Code shows (nothing was answered or sent): {shown:#?}");

    runtime.stop(&sessions, id).unwrap();
    assert!(gone_within(pid, Duration::from_secs(5)));

    // The same through a local provider: no key at all.
    let ollama = configure(own.clone(), &provider("ollama"), "qwen3-coder:30b", None).unwrap();
    approvals.approve(&ollama.approval()).unwrap();
    let local = isolation
        .create(&git, &repo, &claude.id, ollama.model.as_ref(), &[], &[])
        .unwrap();
    let local_id = runtime
        .create(&ollama, local.path.clone(), Some(local.clone()))
        .unwrap();
    let local_pty = runtime
        .run(
            &sessions,
            local_id,
            authorize(&ollama, &trust, &approvals).unwrap(),
            SIZE,
            Arc::new(Screen::default()),
        )
        .unwrap();
    let local_pid = local_pty.pid().unwrap();
    std::thread::sleep(Duration::from_secs(2));
    let (args, env) = process_of(local_pid);
    assert!(args.ends_with("--model qwen3-coder:30b"), "{args}");
    assert_eq!(
        value(&env, "ANTHROPIC_BASE_URL"),
        Some("http://localhost:11434")
    );
    assert_eq!(value(&env, "ANTHROPIC_AUTH_TOKEN"), Some("ollama"));
    assert!(
        !env.iter().any(|w| w.contains(INVALID_KEY)),
        "no Anthropic key for Ollama"
    );
    runtime.stop(&sessions, local_id).unwrap();
    assert!(gone_within(local_pid, Duration::from_secs(5)));
    let detection = x8ai_providers::ollama::detect(
        x8ai_agents::environment::var(&login, "PATH").map(std::ffi::OsStr::new),
        std::net::SocketAddr::from(x8ai_providers::ollama::ADDRESS),
    );
    println!(
        "   Ollama session: BASE_URL localhost, AUTH_TOKEN placeholder, no key; Ollama here: {:?}",
        detection.availability
    );
    assert_ne!(
        detection.availability,
        LocalAvailability::Unavailable,
        "installed with Homebrew here"
    );

    // 12. No secret anywhere on disk the app or the session touched.
    let leaked = files_containing(&base, INVALID_KEY.as_bytes());
    assert_eq!(leaked, Vec::<PathBuf>::new());
    println!("12. key not found in the project, the worktrees, their metadata or the stores");

    // 11. Removing the credential stops the next session from using it.
    keychain.remove("anthropic").unwrap();
    assert!(!after_restart.contains("anthropic").unwrap());
    let credential = after_restart.get("anthropic").unwrap();
    let error = configured("claude-haiku-4-5-20251001", credential.as_ref()).unwrap_err();
    assert!(matches!(error, ConfigureError::MissingCredential(_)));
    println!("11. after removal: {error}");

    // Clean up the worktrees and the throwaway Keychain service.
    for worktree in [&worktree, &local] {
        isolation.remove(&git, &repo, worktree, true).unwrap();
    }
    keychain.remove("anthropic").unwrap();
    assert!(!files_containing(&base, b"sk-ant-x8ai").iter().any(|_| true));
    println!("cleaned up: worktrees removed, Keychain test service empty");
}
