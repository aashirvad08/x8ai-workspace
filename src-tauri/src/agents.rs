//! Agent commands: the IPC face of `x8ai-agents` (docs/agent-runtime.md).
//!
//! The webview can ask which agents exist, ask for an agent to be approved in the
//! open workspace, create an agent session there, and run the agent in it. It
//! names agents and sessions by id only, and a model by provider id and model id.
//! It cannot choose the program, its arguments, its environment or its
//! directory: the program comes from a built-in definition and the user's `PATH`;
//! the model configuration from the agent's adapter and the provider's built-in
//! definition, with the key read here from the Keychain (docs/models.md); the
//! directory is a worktree this module makes (docs/multi-agent.md), or the open
//! workspace itself when it is not a Git repository. Creating a session and every run require the workspace to be
//! trusted and the agent to be approved for it, checked here, natively. Approval is
//! granted only in a native dialog the webview cannot answer. A running agent is a
//! terminal session, driven with the `terminal_*` commands.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use x8ai_agents::adapter::{self, ConfigureError};
use x8ai_agents::discovery::find_executable;
use x8ai_agents::environment::{RESOLVE_TIMEOUT, resolve, var};
use x8ai_agents::isolation::{self, Isolation};
use x8ai_agents::{AgentRuntime, AgentSession, Denied, LaunchPlan, RunError, SessionState, plan};
use x8ai_core::agent::{
    AgentAvailability, AgentChangedFile, AgentChanges, AgentDefinition, AgentList, AgentRemoval,
    AgentSessionId, AgentSessionInfo, AgentSessionState, AgentStatus, AgentWorktree, ChangeKind,
    ProviderSupport, SessionConfiguration, WorkspaceIsolation,
};
use x8ai_core::error::{CommandError, ErrorCode};
use x8ai_core::model::{CredentialState, ModelSelection};
use x8ai_core::terminal::{TerminalInfo, TerminalSize};
use x8ai_core::workspace::FileContent;
use x8ai_git::{FileStatus, Git, Repository};
use x8ai_workspace::Workspace;

use crate::providers::Providers;
use crate::terminal::{ChannelEvents, Terminals, info};
use crate::workspace::Workspaces;

/// Built-in agents, the user's login environment once read, where agent
/// worktrees go, and the agent sessions. Managed Tauri state.
pub struct Agents {
    definitions: Vec<AgentDefinition>,
    runtime: AgentRuntime,
    environment: Mutex<Option<Arc<Resolved>>>,
    isolation: Isolation,
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
            isolation: Isolation::new(isolation::default_root(&home())),
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
        let resolved = Arc::new(match resolve(&shell, &home(), RESOLVE_TIMEOUT) {
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

    fn name_of(&self, agent: &str) -> String {
        self.definitions
            .iter()
            .find(|d| d.id.as_str() == agent)
            .map_or_else(|| agent.to_owned(), |d| d.name.clone())
    }

    /// The agent sessions of workspace `root`, including worktrees left from
    /// earlier runs of the app, which are found again here with their model.
    fn sessions_of(
        &self,
        git: Option<&Git>,
        root: &Path,
        providers: &Providers,
        env: &[(String, String)],
    ) -> Vec<AgentSession> {
        if let Some(git) = git
            && let Ok(Some(repo)) = git.repository(root)
            && let Ok(found) = self.isolation.find(git, &repo)
        {
            let known = self.runtime.sessions_in(root);
            for worktree in found {
                if known
                    .iter()
                    .any(|s| s.worktree.as_ref().is_some_and(|w| w.path == worktree.path))
                {
                    continue;
                }
                let cwd = cwd_in(&worktree.path, &repo);
                let name = self.name_of(worktree.agent.as_str());
                let configuration = configuration_of(
                    providers,
                    worktree.agent.as_str(),
                    worktree.model.as_ref(),
                    env,
                );
                self.runtime
                    .adopt(&name, root, cwd, worktree, configuration);
            }
        }
        self.runtime.sessions_in(root)
    }
}

/// How a session found from an earlier run is configured, until it runs again.
fn configuration_of(
    providers: &Providers,
    agent: &str,
    model: Option<&ModelSelection>,
    env: &[(String, String)],
) -> SessionConfiguration {
    let Some(model) = model else {
        return SessionConfiguration::Agent {
            shell_variables: adapter::shell_variables(agent, env),
        };
    };
    providers
        .definition(model.provider.as_str())
        .ok()
        .and_then(|provider| {
            let credential = providers.credential_state(provider);
            adapter::describe(agent, provider, &model.model, env, credential).ok()
        })
        .unwrap_or_else(|| SessionConfiguration::App {
            provider: model.provider.clone(),
            provider_name: model.provider.to_string(),
            model: model.model.clone(),
            endpoint: String::new(),
            credential: CredentialState::Missing,
            overridden_shell_variables: Vec::new(),
        })
}

/// The launch for `definition` in `root`: with the agent's own configuration, or
/// pointed at `model`, with the provider's key from the Keychain.
fn launch_plan(
    definition: &AgentDefinition,
    environment: &Resolved,
    root: &Path,
    model: Option<&ModelSelection>,
    providers: &Providers,
) -> Result<LaunchPlan, CommandError> {
    let plan = plan(definition, &environment.vars, root).map_err(denied)?;
    let Some(model) = model else {
        return Ok(plan);
    };
    let provider = providers.definition(model.provider.as_str())?;
    // Checked before the key is read: an unsupported choice never touches it.
    adapter::support(definition.id.as_str(), provider).map_err(|reason| {
        configure_error(ConfigureError::Unsupported {
            agent: definition.name.clone(),
            provider: provider.name.clone(),
            reason,
        })
    })?;
    let credential = providers.credential(provider)?;
    adapter::configure(plan, provider, &model.model, credential.as_ref()).map_err(configure_error)
}

/// The `PATH` of the user's login environment, to look for local providers.
pub async fn login_path(app: &AppHandle) -> Result<Option<String>, CommandError> {
    let environment = resolved(app, false).await?;
    Ok(var(&environment.vars, "PATH").map(str::to_owned))
}

/// The user's `git`: the one on their login `PATH`, or the system's.
fn git_in(environment: &Resolved) -> Option<Git> {
    let program = find_executable("git", var(&environment.vars, "PATH"))
        .or_else(|| Some(PathBuf::from("/usr/bin/git")).filter(|p| p.is_file()))?;
    Some(Git::new(program, &environment.vars))
}

/// Where the agent runs in a worktree: the same folder inside it as the workspace
/// is inside its repository, if the worktree has it.
fn cwd_in(worktree: &Path, repo: &Repository) -> PathBuf {
    let inside = worktree.join(repo.prefix.trim_end_matches('/'));
    if repo.prefix.is_empty() || !inside.is_dir() {
        worktree.to_owned()
    } else {
        inside
    }
}

fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
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
    let providers = app.state::<Providers>();
    let root = workspaces.root();
    // Without a workspace, agents are still looked up; they are just not approved.
    let lookup_in = root.clone().unwrap_or_else(|| PathBuf::from("/"));
    let statuses = agents
        .definitions
        .iter()
        .map(|definition| {
            let planned = plan(definition, &environment.vars, &lookup_in);
            let approved = match (&planned, &root) {
                (Ok(plan), Some(_)) => workspaces.is_approved_for_any_provider(plan),
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
                providers: providers
                    .definitions()
                    .iter()
                    .map(|provider| {
                        let support = adapter::support(definition.id.as_str(), provider);
                        ProviderSupport {
                            provider: provider.id.clone(),
                            supported: support.is_ok(),
                            reason: support.err(),
                        }
                    })
                    .collect(),
            }
        })
        .collect();
    let isolation = root
        .as_ref()
        .map(|root| isolation_of(git_in(&environment).as_ref(), root));
    Ok(AgentList {
        agents: statuses,
        environment_problem: environment.problem.clone(),
        isolation,
    })
}

/// How agents would run in `root`.
fn isolation_of(git: Option<&Git>, root: &Path) -> WorkspaceIsolation {
    let unavailable = |reason: &str| WorkspaceIsolation::Unavailable {
        reason: reason.to_owned(),
    };
    let Some(git) = git else {
        return unavailable("Git was not found, so agents cannot get workspaces of their own.");
    };
    match git.repository(root) {
        Ok(Some(Repository {
            head: Some(head),
            branch,
            ..
        })) => WorkspaceIsolation::Worktrees { branch, head },
        Ok(Some(_)) => {
            unavailable("This repository has no commits yet; agent workspaces start from a commit.")
        }
        Ok(None) => unavailable("This folder is not a Git repository."),
        Err(error) => unavailable(&format!("Git could not read this folder: {error}")),
    }
}

/// Makes sure `agent` may run in the open workspace, with its own configuration
/// or pointed at `model`: the workspace must be trusted, and if the agent is not
/// yet approved there (with the executable it would run now, and the provider
/// and endpoint `model` would use), the user is asked in a native dialog.
/// Returns whether it is approved now; `false` if the user declined.
#[tauri::command]
pub async fn agent_request_approval(
    agent: String,
    model: Option<ModelSelection>,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<bool, CommandError> {
    let environment = resolved(&app, false).await?;
    let task_app = app.clone();
    let plan = tauri::async_runtime::spawn_blocking(move || {
        let agents = task_app.state::<Agents>();
        let workspaces = task_app.state::<Workspaces>();
        let definition = agents.definition(&agent)?;
        let root = open_root(&workspaces)?;
        launch_plan(
            definition,
            &environment,
            &root,
            model.as_ref(),
            &task_app.state::<Providers>(),
        )
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))??;
    let workspaces = app.state::<Workspaces>();
    let root = plan.workspace.clone();
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
    let model_line = match &plan.configuration {
        SessionConfiguration::App {
            provider_name,
            endpoint,
            credential,
            ..
        } => format!(
            "Model provider: {provider_name}, at {endpoint}\n{name} will send your code and \
             prompts there{key}. The app replaces any provider settings in your shell for these \
             sessions.\n\n",
            name = plan.name,
            key = match credential {
                CredentialState::InKeychain => ", with your saved API key",
                _ => "",
            },
        ),
        SessionConfiguration::Agent { .. } => format!(
            "Model provider: {}'s own configuration (its settings and your shell)\n\n",
            plan.name
        ),
    };
    let confirmed = window
        .dialog()
        .message(format!(
            "{name} will work in:\n{root}\n(in a Git worktree of its own for each session, \
             when the folder is a Git repository)\n\nProgram: {command}\n{model_line}{name} \
             runs as you, with access to your files, network and credentials, as if you started \
             it in a terminal yourself. This allows it in this folder only. You can revoke it in \
             the Agents panel.",
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

/// Creates a session for `agent` in the open workspace, with the agent's own
/// configuration or pointed at `model`: in a Git repository, a new worktree on a
/// new branch from the checked-out commit; otherwise the folder itself, for one
/// agent at a time. Refused unless the workspace is trusted and the agent
/// approved there for that provider. The user's working tree is not touched.
#[tauri::command]
pub async fn agent_create_session(
    agent: String,
    model: Option<ModelSelection>,
    app: AppHandle,
) -> Result<AgentSessionInfo, CommandError> {
    let environment = resolved(&app, false).await?;
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let agents = task_app.state::<Agents>();
        let workspaces = task_app.state::<Workspaces>();
        let definition = agents.definition(&agent)?;
        let root = open_root(&workspaces)?;
        let plan = launch_plan(
            definition,
            &environment,
            &root,
            model.as_ref(),
            &task_app.state::<Providers>(),
        )?;
        // 1. trust, 2. approval.
        workspaces.authorize(&plan).map_err(denied)?;
        // 3. Git, 4. a worktree of its own.
        let git = git_in(&environment);
        let repo = match &git {
            Some(git) => git.repository(&root).map_err(git_error)?,
            None => None,
        };
        let (cwd, worktree) = match (&git, repo) {
            (Some(git), Some(repo)) => {
                let worktree = agents
                    .isolation
                    .create(git, &repo, &definition.id, plan.model.as_ref())
                    .map_err(isolation_error)?;
                (cwd_in(&worktree.path, &repo), Some(worktree))
            }
            _ => (root.clone(), None),
        };
        let id = agents
            .runtime
            .create(&plan, cwd, worktree)
            .map_err(run_error)?;
        session_info(&agents.runtime.get(id).expect("just created"))
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

/// Runs the agent of `session` (again, after it stopped) on a new terminal
/// session, in the session's own directory, with the session's model. Refused
/// unless the session belongs to the open workspace, the workspace is trusted,
/// the agent approved there for the session's provider, and the provider's key
/// is still saved. Output and exit arrive on `events` as for `terminal_create`.
#[tauri::command]
pub async fn agent_run(
    session: AgentSessionId,
    size: TerminalSize,
    events: Channel,
    app: AppHandle,
) -> Result<TerminalInfo, CommandError> {
    let environment = resolved(&app, false).await?;
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let agents = task_app.state::<Agents>();
        let workspaces = task_app.state::<Workspaces>();
        let terminals = task_app.state::<Terminals>();
        let record = agents
            .runtime
            .get(session)
            .ok_or_else(|| run_error(RunError::NotFound(session)))?;
        let root = open_root(&workspaces)?;
        if record.workspace != root {
            return Err(CommandError::new(
                ErrorCode::PermissionDenied,
                "this agent session belongs to another workspace",
            ));
        }
        let definition = agents.definition(record.agent.as_str())?;
        // The session's model, with the key as saved now: a removed key stops it here.
        let providers = task_app.state::<Providers>();
        let plan = launch_plan(
            definition,
            &environment,
            &root,
            record.model.as_ref(),
            &providers,
        )
        .inspect_err(|error| agents.runtime.fail(session, error.message.clone()))?;
        let authorized = workspaces.authorize(&plan).map_err(|reason| {
            agents.runtime.fail(session, reason.to_string());
            denied(reason)
        })?;
        let pty = agents
            .runtime
            .run(
                terminals.sessions(),
                session,
                authorized,
                size,
                Arc::new(ChannelEvents(events)),
            )
            .map_err(run_error)?;
        Ok(info(&pty))
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

/// The agent sessions of the open workspace, oldest first, including worktrees
/// left from earlier runs of the app.
#[tauri::command]
pub async fn agent_sessions(app: AppHandle) -> Result<Vec<AgentSessionInfo>, CommandError> {
    let environment = resolved(&app, false).await?;
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let agents = task_app.state::<Agents>();
        let Some(root) = task_app.state::<Workspaces>().root() else {
            return Ok(Vec::new());
        };
        agents
            .sessions_of(
                git_in(&environment).as_ref(),
                &root,
                &task_app.state::<Providers>(),
                &environment.vars,
            )
            .iter()
            .map(session_info)
            .collect()
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

/// Stops the agent of `session`. Its worktree stays.
#[tauri::command]
pub fn agent_stop(
    session: AgentSessionId,
    agents: State<'_, Agents>,
    terminals: State<'_, Terminals>,
) -> Result<(), CommandError> {
    agents
        .runtime
        .stop(terminals.sessions(), session)
        .map_err(run_error)
}

/// Removes a stopped agent's session and its worktree. A worktree with
/// uncommitted changes is removed only with `discard`. The agent's branch is kept
/// if it has commits, so nothing committed is ever deleted here.
#[tauri::command]
pub async fn agent_remove(
    session: AgentSessionId,
    discard: bool,
    app: AppHandle,
) -> Result<AgentRemoval, CommandError> {
    let environment = resolved(&app, false).await?;
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let agents = task_app.state::<Agents>();
        let record = agents
            .runtime
            .get(session)
            .ok_or_else(|| run_error(RunError::NotFound(session)))?;
        if record.state == SessionState::Running {
            return Err(run_error(RunError::StillRunning));
        }
        let removal = match &record.worktree {
            None => AgentRemoval {
                kept_branch: None,
                commits: 0,
            },
            Some(worktree) => {
                let git = git_in(&environment)
                    .ok_or_else(|| CommandError::new(ErrorCode::Internal, "Git was not found"))?;
                let repo = git
                    .repository(&record.workspace)
                    .map_err(git_error)?
                    .ok_or_else(|| {
                        CommandError::new(ErrorCode::NotFound, "the repository is gone")
                    })?;
                let removal = agents
                    .isolation
                    .remove(&git, &repo, worktree, discard)
                    .map_err(isolation_error)?;
                AgentRemoval {
                    kept_branch: removal.kept_branch,
                    commits: removal.commits,
                }
            }
        };
        agents.runtime.forget(session).map_err(run_error)?;
        Ok(removal)
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

/// What the agent of `session` changed in its worktree since it started.
#[tauri::command]
pub async fn agent_changes(
    session: AgentSessionId,
    app: AppHandle,
) -> Result<AgentChanges, CommandError> {
    let environment = resolved(&app, false).await?;
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let agents = task_app.state::<Agents>();
        let worktree = worktree_of(&agents, session)?;
        let git = git_in(&environment)
            .ok_or_else(|| CommandError::new(ErrorCode::Internal, "Git was not found"))?;
        let changes = git
            .changes(&worktree.path, &worktree.base)
            .map_err(git_error)?;
        Ok(AgentChanges {
            branch: changes.branch,
            base: changes.base,
            head: changes.head,
            commits: changes.commits,
            uncommitted: changes.uncommitted,
            files: changes
                .files
                .into_iter()
                .map(|f| AgentChangedFile {
                    path: f.path,
                    change: match f.status {
                        FileStatus::Added => ChangeKind::Added,
                        FileStatus::Modified => ChangeKind::Modified,
                        FileStatus::Deleted => ChangeKind::Deleted,
                        FileStatus::Renamed => ChangeKind::Renamed,
                        FileStatus::Untracked => ChangeKind::Untracked,
                        FileStatus::Other => ChangeKind::Other,
                    },
                    from: f.from,
                })
                .collect(),
            diff: changes.diff,
            truncated: changes.truncated,
        })
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

/// Reads a file in the agent's worktree, for inspection. `path` is relative to
/// the worktree root and cannot leave it (the same checks as workspace files).
#[tauri::command]
pub async fn agent_read_file(
    session: AgentSessionId,
    path: String,
    app: AppHandle,
) -> Result<FileContent, CommandError> {
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let agents = task_app.state::<Agents>();
        let worktree = worktree_of(&agents, session)?;
        let scoped = Workspace::open(&worktree.path).map_err(crate::workspace::command_error)?;
        scoped
            .read_text(&path)
            .map_err(crate::workspace::command_error)
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

fn worktree_of(
    agents: &Agents,
    session: AgentSessionId,
) -> Result<x8ai_agents::Worktree, CommandError> {
    let record = agents
        .runtime
        .get(session)
        .ok_or_else(|| run_error(RunError::NotFound(session)))?;
    record.worktree.ok_or_else(|| {
        CommandError::new(
            ErrorCode::InvalidInput,
            "this agent runs directly in your folder, so it has no separate workspace to inspect",
        )
    })
}

fn session_info(session: &AgentSession) -> Result<AgentSessionInfo, CommandError> {
    Ok(AgentSessionInfo {
        id: session.id,
        agent: session.agent.clone(),
        name: session.name.clone(),
        workspace: session.workspace.display().to_string(),
        cwd: session.cwd.display().to_string(),
        worktree: session.worktree.as_ref().map(|w| AgentWorktree {
            branch: w.branch.clone(),
            base: w.base.clone(),
            path: w.path.display().to_string(),
        }),
        started_at: session.started,
        state: match &session.state {
            SessionState::NotRunning => AgentSessionState::NotRunning,
            SessionState::Running => AgentSessionState::Running,
            SessionState::Exited(exit) => AgentSessionState::Exited { exit: exit.clone() },
            SessionState::Failed(message) => AgentSessionState::Failed {
                message: message.clone(),
            },
        },
        terminal: session.terminal,
        configuration: session.configuration.clone(),
    })
}

fn run_error(error: RunError) -> CommandError {
    let code = match &error {
        RunError::NotFound(_) => ErrorCode::NotFound,
        RunError::AlreadyRunning | RunError::StillRunning | RunError::SharedBusy => {
            ErrorCode::Conflict
        }
        RunError::Mismatch | RunError::ModelMismatch | RunError::OutsideWorkspace => {
            ErrorCode::PermissionDenied
        }
        RunError::Pty(_) => ErrorCode::Internal,
    };
    CommandError::new(code, error.to_string())
}

fn isolation_error(error: isolation::Error) -> CommandError {
    let code = match &error {
        isolation::Error::NoCommits => ErrorCode::InvalidInput,
        isolation::Error::HasChanges => ErrorCode::Conflict,
        isolation::Error::Unsafe(_) => ErrorCode::PermissionDenied,
        isolation::Error::Git(_) | isolation::Error::Io { .. } => ErrorCode::Internal,
    };
    CommandError::new(code, error.to_string())
}

fn configure_error(error: ConfigureError) -> CommandError {
    let code = match &error {
        ConfigureError::MissingCredential(_) => ErrorCode::NotFound,
        ConfigureError::Unsupported { .. }
        | ConfigureError::NoAdapter(_)
        | ConfigureError::InvalidModel(_) => ErrorCode::InvalidInput,
    };
    CommandError::new(code, error.to_string())
}

fn git_error(error: x8ai_git::Error) -> CommandError {
    CommandError::new(ErrorCode::Internal, error.to_string())
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
