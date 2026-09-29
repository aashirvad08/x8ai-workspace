//! Launching agents: from a definition to a running process on a PTY session.
//!
//! Three steps, each a separate function so the rules are testable on their own:
//!
//! 1. [`plan`] turns a definition into the exact launch: the executable found on
//!    the user's `PATH`, its arguments, the workspace it runs in and its
//!    environment. Nothing runs yet.
//! 2. [`authorize`] checks that the workspace is trusted and that the user approved
//!    exactly this launch in exactly this workspace. It is the only way to get an
//!    [`Authorized`] launch.
//! 3. [`AgentRuntime::run`] runs an [`Authorized`] launch for an agent session on a
//!    new PTY session in the shared session registry, so an agent is a terminal
//!    session like any other: the same PTY, input, resize, flow control, hangup
//!    and cleanup (`x8ai-pty`).
//!
//! An *agent session* (docs/multi-agent.md) is where an agent works, a worktree of
//! its own or the workspace itself, and the agent running there. It outlives the
//! agent's process: stopping and restarting the agent keeps its worktree. The
//! runtime keeps track of them, so it can report status, keep agents apart, and
//! stop them when their workspace closes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use std::time::{SystemTime, UNIX_EPOCH};

use x8ai_core::agent::{AgentDefinition, AgentSessionId};
use x8ai_core::id::IntegrationId;
use x8ai_core::launch::EnvValue;
use x8ai_core::terminal::{SessionId, TerminalExit, TerminalSize};
use x8ai_pty::{Environment, Program, Session, SessionEvents, Sessions};
use x8ai_workspace::{Approval, ApprovalStore, TrustStore};

use crate::discovery::find_executable;
use crate::environment::var;
use crate::isolation::Worktree;

/// Exactly what would run: shown to the user for approval, and then executed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchPlan {
    pub agent: IntegrationId,
    pub name: String,
    /// The workspace root, canonical. The agent starts in it.
    pub workspace: PathBuf,
    /// Absolute path of the executable, as found on `PATH`.
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl LaunchPlan {
    /// The approval that covers this launch.
    pub fn approval(&self) -> Approval<'_> {
        Approval {
            root: &self.workspace,
            agent: self.agent.as_str(),
            program: &self.program,
            args: &self.args,
        }
    }
}

/// Why an agent cannot start.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Denied {
    #[error("{0} does not run on this operating system")]
    Unsupported(String),
    #[error("{name} is not installed: `{program}` was not found on your PATH")]
    NotInstalled { name: String, program: String },
    #[error("{name} needs the secret {secret}; secrets are not supported yet")]
    NeedsSecret { name: String, secret: String },
    #[error("agents run only in folders you trust; {} is not trusted", .0.display())]
    Untrusted(PathBuf),
    #[error("{name} has not been allowed to run in {}", .workspace.display())]
    NotApproved { name: String, workspace: PathBuf },
}

/// Works out how `definition` would start in `workspace` with the user's login
/// `environment`: the executable is looked up on that environment's `PATH`, and the
/// definition's own variables are added to it.
pub fn plan(
    definition: &AgentDefinition,
    environment: &[(String, String)],
    workspace: &Path,
) -> Result<LaunchPlan, Denied> {
    let name = definition.name.clone();
    if !definition.supports_this_platform() {
        return Err(Denied::Unsupported(name));
    }
    let launch = &definition.launch;
    let program = find_executable(&launch.program, var(environment, "PATH")).ok_or_else(|| {
        Denied::NotInstalled {
            name: name.clone(),
            program: launch.program.clone(),
        }
    })?;
    let mut env = environment.to_vec();
    for extra in &launch.env {
        let value = match &extra.value {
            EnvValue::Literal(value) => value.clone(),
            EnvValue::Secret(secret) => {
                return Err(Denied::NeedsSecret {
                    name,
                    secret: secret.as_str().to_owned(),
                });
            }
        };
        env.retain(|(n, _)| *n != extra.name);
        env.push((extra.name.clone(), value));
    }
    Ok(LaunchPlan {
        agent: definition.id.clone(),
        name,
        workspace: workspace.to_owned(),
        program,
        args: launch.args.clone(),
        env,
    })
}

/// A launch that passed [`authorize`]. Only that function creates one.
#[derive(Debug)]
pub struct Authorized<'a>(&'a LaunchPlan);

/// Allows the launch only if its workspace is trusted and the user approved this
/// agent, with this executable and these arguments, in this workspace.
pub fn authorize<'a>(
    plan: &'a LaunchPlan,
    trust: &TrustStore,
    approvals: &ApprovalStore,
) -> Result<Authorized<'a>, Denied> {
    if !trust.is_trusted(&plan.workspace) {
        return Err(Denied::Untrusted(plan.workspace.clone()));
    }
    if !approvals.is_approved(&plan.approval()) {
        return Err(Denied::NotApproved {
            name: plan.name.clone(),
            workspace: plan.workspace.clone(),
        });
    }
    Ok(Authorized(plan))
}

/// What an agent session is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// Created, stopped, or found from an earlier run of the app.
    NotRunning,
    Running,
    Exited(TerminalExit),
    /// The agent could not be started; says why.
    Failed(String),
}

/// An agent session, as reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    pub id: AgentSessionId,
    pub agent: IntegrationId,
    pub name: String,
    /// The workspace root it belongs to, and was approved for.
    pub workspace: PathBuf,
    /// Where the agent runs.
    pub cwd: PathBuf,
    /// Its worktree; `None` when it runs directly in the workspace.
    pub worktree: Option<Worktree>,
    /// Milliseconds since the Unix epoch.
    pub started: u64,
    pub state: SessionState,
    /// The PTY session while it runs.
    pub terminal: Option<SessionId>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RunError {
    #[error("no agent session {}", .0.0)]
    NotFound(AgentSessionId),
    #[error("the agent is already running in this session")]
    AlreadyRunning,
    #[error("the agent is still running; stop it first")]
    StillRunning,
    #[error(
        "this folder is not a Git repository, so agents cannot get workspaces of their own; \
         one agent at a time runs directly in it, and one is running"
    )]
    SharedBusy,
    #[error("the launch does not belong to this agent session")]
    Mismatch,
    #[error("the agent's directory is not inside its workspace")]
    OutsideWorkspace,
    #[error("{0}")]
    Pty(String),
}

struct Record {
    id: AgentSessionId,
    agent: IntegrationId,
    name: String,
    workspace: PathBuf,
    cwd: PathBuf,
    worktree: Option<Worktree>,
    started: u64,
    pty: Option<Arc<Session>>,
    failure: Option<String>,
}

impl Record {
    fn running(&self) -> bool {
        self.pty.as_ref().is_some_and(|s| !s.has_exited())
    }

    fn view(&self) -> AgentSession {
        let state = match (&self.pty, &self.failure) {
            (Some(pty), _) if !pty.has_exited() => SessionState::Running,
            (Some(pty), _) => pty
                .exit_status()
                .map_or(SessionState::NotRunning, SessionState::Exited),
            (None, Some(message)) => SessionState::Failed(message.clone()),
            (None, None) => SessionState::NotRunning,
        };
        AgentSession {
            id: self.id,
            agent: self.agent.clone(),
            name: self.name.clone(),
            workspace: self.workspace.clone(),
            cwd: self.cwd.clone(),
            worktree: self.worktree.clone(),
            started: self.started,
            terminal: self
                .pty
                .as_ref()
                .filter(|p| !p.has_exited())
                .map(|p| p.id()),
            state,
        }
    }
}

/// The agent sessions of this app run. The PTY sessions themselves live in the
/// shared [`Sessions`] registry, next to the user's shells.
#[derive(Default)]
pub struct AgentRuntime {
    last_id: std::sync::atomic::AtomicU32,
    records: Mutex<Vec<Record>>,
}

impl AgentRuntime {
    /// A new session for the planned agent: in `worktree` (running in `cwd`
    /// inside it), or, without one, directly in the plan's workspace. A workspace
    /// without isolation takes one running agent at a time.
    pub fn create(
        &self,
        plan: &LaunchPlan,
        cwd: PathBuf,
        worktree: Option<Worktree>,
    ) -> Result<AgentSessionId, RunError> {
        let inside = match &worktree {
            Some(worktree) => cwd.starts_with(&worktree.path),
            None => cwd == plan.workspace,
        };
        if !inside {
            return Err(RunError::OutsideWorkspace);
        }
        let mut records = self.lock();
        if worktree.is_none()
            && records
                .iter()
                .any(|r| r.workspace == plan.workspace && r.worktree.is_none() && r.running())
        {
            return Err(RunError::SharedBusy);
        }
        let id = self.next_id();
        records.push(Record {
            id,
            agent: plan.agent.clone(),
            name: plan.name.clone(),
            workspace: plan.workspace.clone(),
            cwd,
            worktree,
            started: now_ms(),
            pty: None,
            failure: None,
        });
        Ok(id)
    }

    /// Adds a session for a worktree found from an earlier run of the app, unless
    /// one already exists for it. Returns the session.
    pub fn adopt(
        &self,
        name: &str,
        workspace: &Path,
        cwd: PathBuf,
        worktree: Worktree,
    ) -> AgentSessionId {
        let mut records = self.lock();
        if let Some(known) = records
            .iter()
            .find(|r| r.worktree.as_ref().is_some_and(|w| w.path == worktree.path))
        {
            return known.id;
        }
        let id = self.next_id();
        records.push(Record {
            id,
            agent: worktree.agent.clone(),
            name: name.to_owned(),
            workspace: workspace.to_owned(),
            cwd,
            started: worktree.created,
            worktree: Some(worktree),
            pty: None,
            failure: None,
        });
        id
    }

    /// Runs the agent of session `id`, in the session's directory, on a new PTY
    /// session. `launch` must be an authorized plan for the same agent and
    /// workspace. Output and exit are reported to `events`.
    pub fn run(
        &self,
        sessions: &Sessions,
        id: AgentSessionId,
        launch: Authorized<'_>,
        size: TerminalSize,
        events: Arc<dyn SessionEvents>,
    ) -> Result<Arc<Session>, RunError> {
        let plan = launch.0;
        let mut records = self.lock();
        let shared_busy = |records: &[Record], record: &Record| {
            record.worktree.is_none()
                && records.iter().any(|r| {
                    r.id != record.id
                        && r.workspace == record.workspace
                        && r.worktree.is_none()
                        && r.running()
                })
        };
        let index = records
            .iter()
            .position(|r| r.id == id)
            .ok_or(RunError::NotFound(id))?;
        let record = &records[index];
        if record.agent != plan.agent || record.workspace != plan.workspace {
            return Err(RunError::Mismatch);
        }
        if record.running() {
            return Err(RunError::AlreadyRunning);
        }
        if shared_busy(&records, record) {
            return Err(RunError::SharedBusy);
        }
        let program = Program::Exec {
            program: plan.program.clone(),
            args: plan.args.iter().map(Into::into).collect(),
            cwd: Some(record.cwd.clone()),
            env: Environment::Exactly(plan.env.clone()),
        };
        let record = &mut records[index];
        match sessions.spawn(&program, size, events) {
            Ok(session) => {
                record.pty = Some(session.clone());
                record.failure = None;
                Ok(session)
            }
            Err(error) => {
                record.pty = None;
                record.failure = Some(error.to_string());
                Err(RunError::Pty(error.to_string()))
            }
        }
    }

    /// Records why the agent could not be started (for example, it was denied).
    pub fn fail(&self, id: AgentSessionId, message: String) {
        if let Some(record) = self.lock().iter_mut().find(|r| r.id == id && !r.running()) {
            record.pty = None;
            record.failure = Some(message);
        }
    }

    /// Hangs up the agent of session `id` (SIGHUP, then SIGKILL after the grace
    /// period). The session and its worktree stay.
    pub fn stop(&self, sessions: &Sessions, id: AgentSessionId) -> Result<(), RunError> {
        let pty = {
            let records = self.lock();
            let record = records
                .iter()
                .find(|r| r.id == id)
                .ok_or(RunError::NotFound(id))?;
            record.pty.as_ref().map(|p| p.id())
        };
        if let Some(pty) = pty {
            // Already closed if the user closed its terminal.
            let _ = sessions.close(pty);
        }
        Ok(())
    }

    pub fn get(&self, id: AgentSessionId) -> Option<AgentSession> {
        self.lock().iter().find(|r| r.id == id).map(Record::view)
    }

    /// Every session, oldest first.
    pub fn sessions(&self) -> Vec<AgentSession> {
        self.lock().iter().map(Record::view).collect()
    }

    /// The sessions of one workspace, oldest first.
    pub fn sessions_in(&self, workspace: &Path) -> Vec<AgentSession> {
        self.lock()
            .iter()
            .filter(|r| r.workspace == workspace)
            .map(Record::view)
            .collect()
    }

    /// The session whose agent runs on PTY session `terminal`.
    pub fn session_of(&self, terminal: SessionId) -> Option<AgentSession> {
        self.lock()
            .iter()
            .find(|r| r.pty.as_ref().is_some_and(|p| p.id() == terminal))
            .map(Record::view)
    }

    /// Whether any agent is still running, so quitting would end it.
    pub fn any_running(&self) -> bool {
        self.lock().iter().any(Record::running)
    }

    /// Forgets a session whose agent is not running, e.g. once its worktree has
    /// been removed.
    pub fn forget(&self, id: AgentSessionId) -> Result<AgentSession, RunError> {
        let mut records = self.lock();
        let index = records
            .iter()
            .position(|r| r.id == id)
            .ok_or(RunError::NotFound(id))?;
        if records[index].running() {
            return Err(RunError::StillRunning);
        }
        Ok(records.remove(index).view())
    }

    /// Stops the agents of every workspace other than `root`, and forgets their
    /// sessions: an agent never outlives the workspace it was approved for. Their
    /// worktrees stay on disk and are found again when that workspace reopens.
    /// Returns how many agents were stopped.
    pub fn stop_outside(&self, sessions: &Sessions, root: &Path) -> usize {
        let leaving: Vec<Record> = {
            let mut records = self.lock();
            let (leaving, kept) = records
                .drain(..)
                .partition::<Vec<_>, _>(|r| r.workspace != root);
            *records = kept;
            leaving
        };
        close_running(sessions, leaving.iter())
    }

    /// Stops every agent running in `root`, e.g. when the folder stops being
    /// trusted. The sessions stay. Returns how many were stopped.
    pub fn stop_in(&self, sessions: &Sessions, root: &Path) -> usize {
        let records = self.lock();
        close_running(sessions, records.iter().filter(|r| r.workspace == root))
    }

    /// Forgets every session, e.g. when the page that owned their terminals
    /// reloads and the terminals are closed with it.
    pub fn forget_all(&self) {
        self.lock().clear();
    }

    fn next_id(&self) -> AgentSessionId {
        AgentSessionId(
            self.last_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                + 1,
        )
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Record>> {
        self.records.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

fn close_running<'a>(sessions: &Sessions, records: impl Iterator<Item = &'a Record>) -> usize {
    let mut stopped = 0;
    for record in records.filter(|r| r.running()) {
        if let Some(pty) = &record.pty {
            // Already gone if the user closed it; nothing left to stop then.
            let _ = sessions.close(pty.id());
            stopped += 1;
        }
    }
    stopped
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
