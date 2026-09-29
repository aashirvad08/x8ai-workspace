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
//! 3. [`AgentRuntime::start`] runs an [`Authorized`] launch on a new session in the
//!    shared session registry, so an agent is a terminal session like any other:
//!    the same PTY, input, resize, flow control, hangup and cleanup (`x8ai-pty`).
//!
//! The runtime keeps track of which sessions are agents and in which workspace, so
//! it can report status and stop agents when their workspace closes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use x8ai_core::agent::AgentDefinition;
use x8ai_core::id::IntegrationId;
use x8ai_core::launch::EnvValue;
use x8ai_core::terminal::{SessionId, TerminalSize};
use x8ai_pty::{Environment, Program, Session, SessionEvents, Sessions};
use x8ai_workspace::{Approval, ApprovalStore, TrustStore};

use crate::discovery::find_executable;
use crate::environment::var;

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

/// Whether an agent session is still running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunState {
    Running,
    Exited,
}

/// A started agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSession {
    pub id: SessionId,
    pub agent: IntegrationId,
    pub workspace: PathBuf,
    pub state: RunState,
}

struct Running {
    agent: IntegrationId,
    workspace: PathBuf,
    session: Arc<Session>,
}

/// The agent sessions the app started. The sessions themselves live in the shared
/// [`Sessions`] registry, next to the user's shells.
#[derive(Default)]
pub struct AgentRuntime {
    running: Mutex<Vec<Running>>,
}

impl AgentRuntime {
    /// Starts an authorized launch on a new PTY session, in its workspace, with
    /// exactly its environment. Output and exit are reported to `events`.
    pub fn start(
        &self,
        sessions: &Sessions,
        launch: Authorized<'_>,
        size: TerminalSize,
        events: Arc<dyn SessionEvents>,
    ) -> Result<Arc<Session>, x8ai_pty::Error> {
        let plan = launch.0;
        let program = Program::Exec {
            program: plan.program.clone(),
            args: plan.args.iter().map(Into::into).collect(),
            cwd: Some(plan.workspace.clone()),
            env: Environment::Exactly(plan.env.clone()),
        };
        let session = sessions.spawn(&program, size, events)?;
        self.lock().push(Running {
            agent: plan.agent.clone(),
            workspace: plan.workspace.clone(),
            session: session.clone(),
        });
        Ok(session)
    }

    /// Every agent session started and not yet forgotten.
    pub fn sessions(&self) -> Vec<AgentSession> {
        self.lock()
            .iter()
            .map(|r| AgentSession {
                id: r.session.id(),
                agent: r.agent.clone(),
                workspace: r.workspace.clone(),
                state: if r.session.has_exited() {
                    RunState::Exited
                } else {
                    RunState::Running
                },
            })
            .collect()
    }

    pub fn status(&self, id: SessionId) -> Option<AgentSession> {
        self.sessions().into_iter().find(|s| s.id == id)
    }

    /// Whether any agent is still running, so quitting would end it.
    pub fn any_running(&self) -> bool {
        self.prune();
        !self.lock().is_empty()
    }

    /// Hangs up an agent session (SIGHUP, then SIGKILL after the grace period).
    pub fn stop(&self, sessions: &Sessions, id: SessionId) -> Result<(), x8ai_pty::Error> {
        self.lock().retain(|r| r.session.id() != id);
        sessions.close(id)
    }

    /// Stops every agent whose workspace is not `root`: an agent never outlives
    /// the workspace it was approved for. Returns how many were stopped.
    pub fn stop_outside(&self, sessions: &Sessions, root: &Path) -> usize {
        self.stop_where(sessions, |workspace| workspace != root)
    }

    /// Stops every agent running in `root`, e.g. when the folder stops being
    /// trusted. Returns how many were stopped.
    pub fn stop_in(&self, sessions: &Sessions, root: &Path) -> usize {
        self.stop_where(sessions, |workspace| workspace == root)
    }

    fn stop_where(&self, sessions: &Sessions, stop: impl Fn(&Path) -> bool) -> usize {
        let stopped: Vec<SessionId> = {
            let mut running = self.lock();
            let (stopping, kept) = running
                .drain(..)
                .partition::<Vec<_>, _>(|r| stop(&r.workspace));
            *running = kept;
            stopping.iter().map(|r| r.session.id()).collect()
        };
        for id in &stopped {
            // Already gone if the user closed it; nothing left to stop then.
            let _ = sessions.close(*id);
        }
        stopped.len()
    }

    /// Forgets every agent, e.g. when the page that owned their sessions reloads
    /// and the sessions are closed with it.
    pub fn forget_all(&self) {
        self.lock().clear();
    }

    /// Forgets agents that have exited.
    fn prune(&self) {
        self.lock().retain(|r| !r.session.has_exited());
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Running>> {
        self.running.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
