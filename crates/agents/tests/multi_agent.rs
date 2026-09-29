//! Several agents at once in one Git repository, each in a worktree of its own
//! (docs/multi-agent.md). A script stands in for the agents; it reports where it
//! runs, writes files, commits, and exits when told to.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nix::sys::signal::kill;
use nix::unistd::Pid;
use x8ai_agents::isolation::Error as IsolationError;
use x8ai_agents::{AgentRuntime, Isolation, LaunchPlan, SessionState, Worktree, authorize, plan};
use x8ai_core::agent::{AgentDefinition, AgentSessionId, SessionConfiguration};
use x8ai_core::id::IntegrationId;
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_git::{FileStatus, Git, Repository};
use x8ai_pty::{Session, SessionEvents, Sessions};
use x8ai_workspace::{ApprovalStore, TrustStore};

const SIZE: TerminalSize = TerminalSize {
    cols: 100,
    rows: 30,
};
const TIMEOUT: Duration = Duration::from_secs(15);

const FAKE_AGENT: &str = r#"#!/bin/sh
echo "cwd=$(pwd)"
echo "ready"
while read -r command argument; do
  case "$command" in
    write) echo "work by $AGENT_NAME" > "$argument"; echo "wrote $argument" ;;
    commit) git add -A >/dev/null && git commit -qm "work by $AGENT_NAME" && echo "committed" ;;
    quit) exit 0 ;;
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
}

fn sh_git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
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
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

struct Fixture {
    _temp: tempfile::TempDir,
    root: PathBuf,
    bin: PathBuf,
    isolation: Isolation,
    isolation_root: PathBuf,
    git: Git,
    trust: TrustStore,
    approvals: ApprovalStore,
    sessions: Sessions,
    runtime: AgentRuntime,
}

/// A trusted repository with one commit, and two fake agents on "PATH".
fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let base = fs::canonicalize(temp.path()).unwrap();
    let root = base.join("project");
    let bin = base.join("bin");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir(&bin).unwrap();
    fs::write(root.join("README.md"), "# project\n").unwrap();
    fs::write(root.join("src/lib.rs"), "pub fn answer() -> u32 { 42 }\n").unwrap();
    sh_git(&root, &["init", "-q"]);
    sh_git(&root, &["add", "."]);
    sh_git(&root, &["commit", "-qm", "first"]);
    for name in ["agent-one", "agent-two"] {
        let path = bin.join(name);
        fs::write(&path, FAKE_AGENT).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    let isolation_root = base.join("home/.x8ai/worktrees");
    let (mut trust, _) = TrustStore::load(base.join("data/trusted.json"));
    trust.set(&root, true).unwrap();
    let (approvals, _) = ApprovalStore::load(base.join("data/approvals.json"));
    Fixture {
        _temp: temp,
        isolation: Isolation::new(isolation_root.clone()),
        isolation_root,
        git: Git::new(PathBuf::from("git"), &std::env::vars().collect::<Vec<_>>()),
        root,
        bin,
        trust,
        approvals,
        sessions: Sessions::default(),
        runtime: AgentRuntime::default(),
    }
}

fn definition(id: &str) -> AgentDefinition {
    serde_json::from_value(serde_json::json!({
        "id": id,
        "name": id,
        "launch": { "program": id },
        "capabilities": {}
    }))
    .unwrap()
}

impl Fixture {
    fn repo(&self) -> Repository {
        self.git.repository(&self.root).unwrap().unwrap()
    }

    fn plan(&mut self, agent: &str) -> LaunchPlan {
        let env = vec![
            (
                "PATH".into(),
                format!("{}:/usr/bin:/bin", self.bin.display()),
            ),
            (
                "HOME".into(),
                self.root.parent().unwrap().display().to_string(),
            ),
            ("AGENT_NAME".into(), agent.into()),
            ("GIT_AUTHOR_NAME".into(), "Agent".into()),
            ("GIT_AUTHOR_EMAIL".into(), "agent@example.com".into()),
            ("GIT_COMMITTER_NAME".into(), "Agent".into()),
            ("GIT_COMMITTER_EMAIL".into(), "agent@example.com".into()),
        ];
        let plan = plan(&definition(agent), &env, &self.root).unwrap();
        self.approvals.approve(&plan.approval()).unwrap();
        plan
    }

    /// What the app does on Launch in a repository: a worktree, then a session in it.
    fn create(&mut self, agent: &str) -> (LaunchPlan, AgentSessionId, Worktree) {
        let plan = self.plan(agent);
        let repo = self.repo();
        let worktree = self
            .isolation
            .create(
                &self.git,
                &repo,
                &IntegrationId::new(agent).unwrap(),
                None,
                &[],
            )
            .unwrap();
        let id = self
            .runtime
            .create(&plan, worktree.path.clone(), Some(worktree.clone()))
            .unwrap();
        (plan, id, worktree)
    }

    fn run(&self, plan: &LaunchPlan, id: AgentSessionId) -> (Arc<Session>, Arc<Recorder>) {
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

fn own_configuration() -> SessionConfiguration {
    SessionConfiguration::Agent {
        shell_variables: Vec::new(),
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
fn two_agents_run_at_once_each_in_a_worktree_of_its_own() {
    let mut f = fixture();
    let (plan_one, one, tree_one) = f.create("agent-one");
    let (plan_two, two, tree_two) = f.create("agent-two");
    let (session_one, output_one) = f.run(&plan_one, one);
    let (session_two, output_two) = f.run(&plan_two, two);

    assert_ne!(tree_one.path, tree_two.path);
    assert_ne!(tree_one.branch, tree_two.branch);
    assert!(
        output_one
            .output()
            .contains(&format!("cwd={}", tree_one.path.display()))
    );
    assert!(
        output_two
            .output()
            .contains(&format!("cwd={}", tree_two.path.display()))
    );
    assert!(tree_one.branch.starts_with("agent/agent-one/"));
    assert!(tree_two.branch.starts_with("agent/agent-two/"));

    // Both running, independently, each associated with its own worktree.
    let sessions = f.runtime.sessions_in(&f.root);
    assert_eq!(sessions.len(), 2);
    for (id, tree, pty) in [
        (one, &tree_one, &session_one),
        (two, &tree_two, &session_two),
    ] {
        let session = f.runtime.get(id).unwrap();
        assert_eq!(session.state, SessionState::Running);
        assert_eq!(session.cwd, tree.path);
        assert_eq!(session.worktree.as_ref(), Some(tree));
        assert_eq!(session.terminal, Some(pty.id()));
    }
    f.sessions.shutdown(Duration::from_millis(500));
}

#[test]
fn agents_work_in_their_worktrees_and_the_users_tree_stays_as_it_was() {
    let mut f = fixture();
    let head = sh_git(&f.root, &["rev-parse", "HEAD"]);
    let (plan_one, one, tree_one) = f.create("agent-one");
    let (plan_two, two, tree_two) = f.create("agent-two");
    let (session_one, output_one) = f.run(&plan_one, one);
    let (session_two, output_two) = f.run(&plan_two, two);

    session_one.write(b"write README.md\n".to_vec()).unwrap();
    output_one.wait_for("wrote README.md");
    session_one.write(b"commit\n".to_vec()).unwrap();
    output_one.wait_for("committed");
    session_two.write(b"write notes.txt\n".to_vec()).unwrap();
    output_two.wait_for("wrote notes.txt");

    // The user's working tree, branch and commit are untouched.
    assert_eq!(
        fs::read_to_string(f.root.join("README.md")).unwrap(),
        "# project\n"
    );
    assert!(!f.root.join("notes.txt").exists());
    assert!(f.git.is_clean(&f.root).unwrap());
    assert_eq!(sh_git(&f.root, &["rev-parse", "HEAD"]), head);
    assert_eq!(
        sh_git(&f.root, &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );

    // Each agent's work is in its own worktree only, and the diff shows it.
    let changes_one = f.git.changes(&tree_one.path, &tree_one.base).unwrap();
    assert_eq!(changes_one.commits, 1);
    assert!(!changes_one.uncommitted);
    assert_eq!(changes_one.files.len(), 1);
    assert_eq!(changes_one.files[0].path, "README.md");
    assert_eq!(changes_one.files[0].status, FileStatus::Modified);
    assert!(changes_one.diff.contains("+work by agent-one"));
    let changes_two = f.git.changes(&tree_two.path, &tree_two.base).unwrap();
    assert_eq!(changes_two.commits, 0);
    assert!(changes_two.uncommitted);
    assert_eq!(changes_two.files.len(), 1);
    assert_eq!(changes_two.files[0].path, "notes.txt");
    assert_eq!(changes_two.files[0].status, FileStatus::Untracked);
    assert_eq!(
        fs::read_to_string(tree_two.path.join("README.md")).unwrap(),
        "# project\n"
    );
    f.sessions.shutdown(Duration::from_millis(500));
}

#[test]
fn worktrees_are_made_only_under_the_apps_directory_for_this_repository() {
    let mut f = fixture();
    let repo = f.repo();
    let (_, _, tree) = f.create("agent-one");
    let dir = f.isolation.repository_dir(&repo);
    assert!(dir.starts_with(&f.isolation_root));
    assert_eq!(tree.path.parent(), Some(dir.as_path()));
    assert!(
        !tree.path.starts_with(&f.root),
        "a worktree inside the user's project"
    );
    let name = tree.path.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("agent-one-"));
    assert!(
        name.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    );
    // Git's own record of it is in the repository, where Git keeps every worktree's.
    assert!(repo.common_dir.join("worktrees").join(name).is_dir());
    // Nothing was added to the user's working tree.
    assert!(f.git.is_clean(&f.root).unwrap());
}

#[test]
fn a_symlinked_worktree_directory_is_refused() {
    let mut f = fixture();
    let repo = f.repo();
    let elsewhere = f.root.parent().unwrap().join("elsewhere");
    fs::create_dir(&elsewhere).unwrap();
    let dir = f.isolation.repository_dir(&repo);
    fs::create_dir_all(dir.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
    let _ = f.plan("agent-one");

    let result = f.isolation.create(
        &f.git,
        &repo,
        &IntegrationId::new("agent-one").unwrap(),
        None,
        &[],
    );
    assert!(
        matches!(result, Err(IsolationError::Unsafe(_))),
        "{result:?}"
    );
    assert_eq!(fs::read_dir(&elsewhere).unwrap().count(), 0);
}

#[test]
fn stopping_one_agent_leaves_the_other_running() {
    let mut f = fixture();
    let (plan_one, one, _) = f.create("agent-one");
    let (plan_two, two, _) = f.create("agent-two");
    let (session_one, _) = f.run(&plan_one, one);
    let (session_two, _) = f.run(&plan_two, two);

    f.runtime.stop(&f.sessions, one).unwrap();
    assert!(wait_until_gone(session_one.pid().unwrap()));
    assert!(alive(session_two.pid().unwrap()));
    assert_eq!(f.runtime.get(two).unwrap().state, SessionState::Running);
    assert_ne!(f.runtime.get(one).unwrap().state, SessionState::Running);
    f.sessions.shutdown(Duration::from_millis(500));
}

#[test]
fn a_restarted_agent_is_back_in_the_same_worktree_with_its_work() {
    let mut f = fixture();
    let (plan, id, tree) = f.create("agent-one");
    let (session, output) = f.run(&plan, id);
    session.write(b"write draft.txt\n".to_vec()).unwrap();
    output.wait_for("wrote draft.txt");
    session.write(b"quit\n".to_vec()).unwrap();
    assert!(session.wait_for_exit(TIMEOUT));

    let (again, output) = f.run(&plan, id);
    assert!(
        output
            .output()
            .contains(&format!("cwd={}", tree.path.display()))
    );
    assert!(tree.path.join("draft.txt").exists());
    assert_eq!(f.runtime.get(id).unwrap().terminal, Some(again.id()));
    f.sessions.shutdown(Duration::from_millis(500));
}

#[test]
fn quitting_the_app_ends_every_agent_and_keeps_their_worktrees() {
    let mut f = fixture();
    let (plan_one, one, tree_one) = f.create("agent-one");
    let (plan_two, two, tree_two) = f.create("agent-two");
    let (session_one, _) = f.run(&plan_one, one);
    let (session_two, _) = f.run(&plan_two, two);

    f.sessions.shutdown(Duration::from_millis(500));
    assert!(wait_until_gone(session_one.pid().unwrap()));
    assert!(wait_until_gone(session_two.pid().unwrap()));
    assert!(!f.runtime.any_running());
    // Their work is not thrown away with the app.
    assert!(tree_one.path.is_dir() && tree_two.path.is_dir());

    // After a restart (a new runtime), the worktrees are found again.
    let found = Isolation::new(f.isolation_root.clone())
        .find(&f.git, &f.repo())
        .unwrap();
    let paths: Vec<&Path> = found.iter().map(|w| w.path.as_path()).collect();
    assert_eq!(
        paths,
        vec![tree_one.path.as_path(), tree_two.path.as_path()]
    );
    assert_eq!(found[0].branch, tree_one.branch);
    assert_eq!(found[0].base, tree_one.base);
    let runtime = AgentRuntime::default();
    let adopted = runtime.adopt(
        "agent-one",
        &f.root,
        found[0].path.clone(),
        found[0].clone(),
        own_configuration(),
    );
    assert_eq!(
        runtime.get(adopted).unwrap().state,
        SessionState::NotRunning
    );
    assert_eq!(
        runtime.adopt(
            "agent-one",
            &f.root,
            found[0].path.clone(),
            found[0].clone(),
            own_configuration()
        ),
        adopted
    );
}

#[test]
fn removing_a_worktree_never_throws_away_work_silently() {
    let mut f = fixture();
    let repo = f.repo();
    let (plan, id, tree) = f.create("agent-one");
    let (session, output) = f.run(&plan, id);
    session.write(b"write unsaved.txt\n".to_vec()).unwrap();
    output.wait_for("wrote unsaved.txt");
    session.write(b"quit\n".to_vec()).unwrap();
    assert!(session.wait_for_exit(TIMEOUT));

    // Uncommitted work: refused unless the user says to discard it.
    assert!(matches!(
        f.isolation.remove(&f.git, &repo, &tree, false),
        Err(IsolationError::HasChanges)
    ));
    assert!(tree.path.is_dir());
    let removal = f.isolation.remove(&f.git, &repo, &tree, true).unwrap();
    assert!(!tree.path.exists());
    // Nothing was committed, so the branch goes too.
    assert_eq!(removal.kept_branch, None);
    assert!(!f.git.branch_exists(&repo, &tree.branch).unwrap());
    f.runtime.forget(id).unwrap();
    assert!(f.isolation.find(&f.git, &repo).unwrap().is_empty());
}

#[test]
fn removing_a_worktree_keeps_the_branch_with_the_agents_commits() {
    let mut f = fixture();
    let repo = f.repo();
    let (plan, id, tree) = f.create("agent-one");
    let (session, output) = f.run(&plan, id);
    session.write(b"write feature.txt\n".to_vec()).unwrap();
    output.wait_for("wrote feature.txt");
    session.write(b"commit\n".to_vec()).unwrap();
    output.wait_for("committed");

    // A running agent's session is not forgotten.
    assert!(f.runtime.forget(id).is_err());
    session.write(b"quit\n".to_vec()).unwrap();
    assert!(session.wait_for_exit(TIMEOUT));

    let removal = f.isolation.remove(&f.git, &repo, &tree, false).unwrap();
    assert_eq!(removal.kept_branch.as_deref(), Some(tree.branch.as_str()));
    assert_eq!(removal.commits, 1);
    assert!(!tree.path.exists());
    assert!(f.git.branch_exists(&repo, &tree.branch).unwrap());
    let message = sh_git(&f.root, &["log", "-1", "--format=%s", &tree.branch]);
    assert_eq!(message, "work by agent-one");
}

#[test]
fn launching_still_requires_trust_and_approval() {
    let mut f = fixture();
    let (plan, id, _) = f.create("agent-one");
    f.approvals.revoke(&f.root, "agent-one").unwrap();
    assert!(authorize(&plan, &f.trust, &f.approvals).is_err());
    f.approvals.approve(&plan.approval()).unwrap();
    f.trust.set(&f.root, false).unwrap();
    assert!(authorize(&plan, &f.trust, &f.approvals).is_err());
    assert_eq!(f.runtime.get(id).unwrap().state, SessionState::NotRunning);
}

#[test]
fn a_repository_without_commits_cannot_be_isolated() {
    let f = fixture();
    let fresh = f.root.parent().unwrap().join("fresh");
    fs::create_dir(&fresh).unwrap();
    sh_git(&fresh, &["init", "-q"]);
    let repo = f.git.repository(&fresh).unwrap().unwrap();
    let result = f.isolation.create(
        &f.git,
        &repo,
        &IntegrationId::new("agent-one").unwrap(),
        None,
        &[],
    );
    assert!(
        matches!(result, Err(IsolationError::NoCommits)),
        "{result:?}"
    );
}

#[test]
fn metadata_that_was_not_made_by_the_app_is_ignored() {
    let mut f = fixture();
    let repo = f.repo();
    let (_, _, real) = f.create("agent-one");
    let dir = f.isolation.repository_dir(&repo);
    // Crafted entries: a path outside, a name that does not match, a bad revision.
    let base = &real.base;
    let crafted = [
        ("..-x.json", format!(r#"{{"version":1,"agent":"..","token":"x","base":"{base}","created":0}}"#)),
        ("agent-one-20260101-000000-aaaaaa.json", format!(r#"{{"version":1,"agent":"agent-one","token":"20260101-000000-bbbbbb","base":"{base}","created":0}}"#)),
        ("agent-one-20260101-000000-cccccc.json", r#"{"version":1,"agent":"agent-one","token":"20260101-000000-cccccc","base":"--output=/tmp/x","created":0}"#.to_owned()),
    ];
    for (name, json) in crafted {
        fs::write(dir.join(name), json).unwrap();
    }
    let found = f.isolation.find(&f.git, &repo).unwrap();
    assert_eq!(found, vec![real]);
}

#[test]
fn a_worktree_that_cannot_be_made_leaves_nothing_behind() {
    let f = fixture();
    // A well-formed commit id that is not in the repository: `git worktree add` fails.
    let repo = Repository {
        head: Some("0".repeat(40)),
        ..f.repo()
    };
    let result = f.isolation.create(
        &f.git,
        &repo,
        &IntegrationId::new("agent-one").unwrap(),
        None,
        &[],
    );
    assert!(result.is_err());
    let dir = f.isolation.repository_dir(&repo);
    let leftovers: Vec<_> = fs::read_dir(&dir)
        .map(|d| d.flatten().collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    assert_eq!(sh_git(&f.root, &["branch", "--list", "agent/*"]), "");
}
