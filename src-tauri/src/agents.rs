//! Agent commands: the IPC face of `x8ai-agents` (docs/agent-runtime.md).
//!
//! The webview can ask which agents exist, ask for an agent to be approved in the
//! open workspace, and start an approved agent there. It cannot choose the
//! program, its arguments, its environment or its directory: the program comes
//! from a built-in definition and the user's `PATH`, the directory is the open
//! workspace. Starting requires the workspace to be trusted and the agent to be
//! approved for it; both are checked here, natively, on every start. Approval is
//! granted only in a native dialog the webview cannot answer. Once started, an
//! agent is a terminal session, driven with the `terminal_*` commands.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use x8ai_agents::environment::{RESOLVE_TIMEOUT, resolve};
use x8ai_agents::{AgentRuntime, Denied, LaunchPlan, plan};
use x8ai_core::agent::{AgentAvailability, AgentDefinition, AgentList, AgentStatus};
use x8ai_core::error::{CommandError, ErrorCode};
use x8ai_core::terminal::{TerminalInfo, TerminalSize};

use crate::terminal::{ChannelEvents, Terminals, info};
use crate::workspace::Workspaces;

/// Built-in agents, the user's login environment once read, and the agent
/// sessions started. Managed Tauri state.
pub struct Agents {
    definitions: Vec<AgentDefinition>,
    runtime: AgentRuntime,
    environment: Mutex<Option<Arc<Resolved>>>,
}

/// The environment agents are found and started with.
struct Resolved {
    vars: Vec<(String, String)>,
    /// Why the login environment could not be read, if it could not.
    problem: Option<String>,
}

impl Default for Agents {
    fn default() -> Self {
        Self {
            definitions: x8ai_agents::builtin(),
            runtime: AgentRuntime::default(),
            environment: Mutex::new(None),
        }
    }
}

impl Agents {
    /// Whether any agent is still running, so quitting would end it.
    pub fn any_running(&self) -> bool {
        self.runtime.any_running()
    }

    /// Stops agents started in any workspace other than `root`, which just became
    /// the open one. An agent never outlives the workspace it was approved for.
    pub fn stop_outside(&self, terminals: &Terminals, root: &Path) {
        self.runtime.stop_outside(terminals.sessions(), root);
    }

    /// Stops agents running in `root`, which is no longer trusted.
    pub fn stop_in(&self, terminals: &Terminals, root: &Path) {
        self.runtime.stop_in(terminals.sessions(), root);
    }

    /// Called when the page reloads; its sessions are closed with it.
    pub fn forget_all(&self) {
        self.runtime.forget_all();
    }

    fn definition(&self, id: &str) -> Result<&AgentDefinition, CommandError> {
        self.definitions
            .iter()
            .find(|d| d.id.as_str() == id)
            .ok_or_else(|| CommandError::new(ErrorCode::NotFound, format!("no agent {id:?}")))
    }

    /// The login environment, read once and kept, or again when `refresh` is set.
    /// Runs the user's shell, so call it off the IPC runtime (see `resolved`).
    fn environment(&self, refresh: bool) -> Arc<Resolved> {
        if !refresh && let Some(resolved) = lock(&self.environment).as_ref() {
            return resolved.clone();
        }
        let shell = PathBuf::from(x8ai_pty::user_shell());
        let home = std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let resolved = Arc::new(match resolve(&shell, &home, RESOLVE_TIMEOUT) {
            Ok(vars) => Resolved {
                vars,
                problem: None,
            },
            Err(error) => Resolved {
                vars: std::env::vars().collect(),
                problem: Some(error.to_string()),
            },
        });
        *lock(&self.environment) = Some(resolved.clone());
        resolved
    }
}

/// Reads (or returns the kept) login environment on a blocking thread.
async fn resolved(app: &AppHandle, refresh: bool) -> Result<Arc<Resolved>, CommandError> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || app.state::<Agents>().environment(refresh))
        .await
        .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))
}

/// Every built-in agent: whether it is installed here, and whether it is
/// approved for the open workspace. With `refresh`, the login environment is read
/// again first (after installing an agent, for example).
#[tauri::command]
pub async fn agent_list(refresh: bool, app: AppHandle) -> Result<AgentList, CommandError> {
    let environment = resolved(&app, refresh).await?;
    let agents = app.state::<Agents>();
    let workspaces = app.state::<Workspaces>();
    let root = workspaces.root();
    // Without a workspace, agents are still looked up; they are just not approved.
    let lookup_in = root.clone().unwrap_or_else(|| PathBuf::from("/"));
    let statuses = agents
        .definitions
        .iter()
        .map(|definition| {
            let planned = plan(definition, &environment.vars, &lookup_in);
            let approved = match (&planned, &root) {
                (Ok(plan), Some(_)) => workspaces.is_approved(plan),
                _ => false,
            };
            let availability = match planned {
                Ok(plan) => AgentAvailability::Installed {
                    executable: plan.program.display().to_string(),
                },
                Err(Denied::NotInstalled { program, .. }) => {
                    AgentAvailability::NotInstalled { program }
                }
                Err(_) => AgentAvailability::Unsupported,
            };
            AgentStatus {
                id: definition.id.clone(),
                name: definition.name.clone(),
                description: definition.description.clone(),
                availability,
                approved,
            }
        })
        .collect();
    Ok(AgentList {
        agents: statuses,
        environment_problem: environment.problem.clone(),
    })
}

/// Makes sure `agent` may run in the open workspace: the workspace must be
/// trusted, and if the agent is not yet approved there (with the executable it
/// would run now), the user is asked in a native dialog. Returns whether it is
/// approved now; `false` if the user declined.
#[tauri::command]
pub async fn agent_request_approval(
    agent: String,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<bool, CommandError> {
    let environment = resolved(&app, false).await?;
    let agents = app.state::<Agents>();
    let workspaces = app.state::<Workspaces>();
    let definition = agents.definition(&agent)?;
    let root = open_root(&workspaces)?;
    let plan = plan(definition, &environment.vars, &root).map_err(denied)?;
    if !workspaces.is_trusted(&root) {
        return Err(denied(Denied::Untrusted(root)));
    }
    if workspaces.is_approved(&plan) {
        return Ok(true);
    }

    let folder = root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let command = std::iter::once(plan.program.display().to_string())
        .chain(plan.args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ");
    let confirmed = window
        .dialog()
        .message(format!(
            "{name} will start in:\n{root}\n\nProgram: {command}\n\n{name} runs as you, with \
             access to your files, network and credentials, as if you started it in a \
             terminal yourself. This allows it in this folder only. You can revoke it in the \
             Agents panel.",
            name = plan.name,
            root = root.display(),
        ))
        .title(format!("Allow {} to work in “{folder}”?", plan.name))
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Allow".into(),
            "Cancel".into(),
        ))
        .parent(&window)
        .blocking_show();
    if !confirmed {
        return Ok(false);
    }
    // The dialog may have been open for a while: approve only if the same
    // workspace is still open and still trusted.
    if workspaces.root().as_deref() != Some(root.as_path()) || !workspaces.is_trusted(&root) {
        return Err(CommandError::new(
            ErrorCode::Conflict,
            "the workspace changed while the approval was open; nothing was approved",
        ));
    }
    workspaces.approve(&plan)?;
    Ok(true)
}

/// Forgets `agent`'s approval in the open workspace. Running sessions continue.
#[tauri::command]
pub fn agent_revoke(
    agent: String,
    agents: State<'_, Agents>,
    workspaces: State<'_, Workspaces>,
) -> Result<(), CommandError> {
    let definition = agents.definition(&agent)?;
    let root = open_root(&workspaces)?;
    workspaces.revoke(&root, definition.id.as_str())
}

/// Starts `agent` in the open workspace, on a new terminal session. Refused unless
/// the workspace is trusted and the agent is approved for it with the executable
/// it would run now. Output and exit arrive on `events` as for `terminal_create`.
#[tauri::command]
pub async fn agent_start(
    agent: String,
    size: TerminalSize,
    events: Channel,
    app: AppHandle,
) -> Result<TerminalInfo, CommandError> {
    let environment = resolved(&app, false).await?;
    let agents = app.state::<Agents>();
    let workspaces = app.state::<Workspaces>();
    let terminals = app.state::<Terminals>();
    let definition = agents.definition(&agent)?;
    let root = open_root(&workspaces)?;
    let plan: LaunchPlan = plan(definition, &environment.vars, &root).map_err(denied)?;
    let authorized = workspaces.authorize(&plan).map_err(denied)?;
    let session = agents
        .runtime
        .start(
            terminals.sessions(),
            authorized,
            size,
            Arc::new(ChannelEvents(events)),
        )
        .map_err(crate::terminal::command_error)?;
    Ok(info(&session))
}

fn open_root(workspaces: &Workspaces) -> Result<PathBuf, CommandError> {
    workspaces
        .root()
        .ok_or_else(|| CommandError::new(ErrorCode::NotFound, "no workspace is open"))
}

fn denied(reason: Denied) -> CommandError {
    let code = match &reason {
        Denied::Untrusted(_) | Denied::NotApproved { .. } => ErrorCode::PermissionDenied,
        Denied::NotInstalled { .. } => ErrorCode::NotFound,
        Denied::Unsupported(_) | Denied::NeedsSecret { .. } => ErrorCode::InvalidInput,
    };
    CommandError::new(code, reason.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
