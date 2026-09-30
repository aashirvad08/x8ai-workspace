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
//! workspace itself when it is not a Git repository. A session's MCP servers are
//! chosen natively from the registry (docs/mcp.md): the webview names only the
//! session-scoped servers it wants, by id; stdio servers are started by this
//! module, for the session, behind a private socket, after the same trust and
//! approval checks. Creating a session and every run require the workspace to be
//! trusted and the agent to be approved for it, checked here, natively. Approval is
//! granted only in a native dialog the webview cannot answer. A running agent is a
//! terminal session, driven with the `terminal_*` commands.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use x8ai_agents::adapter::{self, AgentMcpServer, AgentMcpTransport, AgentSkill, ConfigureError};
use x8ai_agents::discovery::find_executable;
use x8ai_agents::environment::{RESOLVE_TIMEOUT, resolve, var};
use x8ai_agents::isolation::{self, Isolation};
use x8ai_agents::{AgentRuntime, AgentSession, Denied, LaunchPlan, RunError, SessionState, plan};
use x8ai_core::agent::{
    AgentChangedFile, AgentChanges, AgentDefinition, AgentList, AgentRemoval, AgentSessionId,
    AgentSessionInfo, AgentSessionState, AgentStatus, AgentWorktree, ChangeKind,
    SessionConfiguration, WorkspaceIsolation,
};
use x8ai_core::error::{CommandError, ErrorCode};
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{McpEnvSource, McpServerTransport};
use x8ai_core::model::{CredentialState, ModelSelection};
use x8ai_core::skill::{Skill, SkillRef};
use x8ai_core::terminal::TerminalExit;
use x8ai_core::terminal::{TerminalInfo, TerminalSize};
use x8ai_core::workspace::FileContent;
use x8ai_git::{FileStatus, Git, Repository};
use x8ai_mcp::{Launch, MaterialTransport, Prepared};
use x8ai_pty::SessionEvents;
use x8ai_workspace::Workspace;

use crate::mcp::{Mcp, RunServer};
use crate::providers::Providers;
use crate::skills::Skills;
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
pub(crate) struct Resolved {
    pub(crate) vars: Vec<(String, String)>,
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

/// A launch's MCP servers: every one attached to the session, those that will
/// run, exactly as they would, and the others with the reason.
struct McpSelection {
    attached: Vec<IntegrationId>,
    prepared: Vec<Prepared>,
    skipped: Vec<(IntegrationId, String)>,
}

/// The servers a new session of `definition` in `root` gets: every enabled global
/// and workspace server the agent can use, and the session servers `chosen`.
/// Choosing servers for an agent that cannot use them is refused; the others are
/// simply not attached to it.
fn mcp_for_new_session(
    mcp: &Mcp,
    definition: &AgentDefinition,
    root: &Path,
    chosen: &[String],
    path: Option<&str>,
) -> Result<McpSelection, CommandError> {
    let chosen: Vec<IntegrationId> = chosen
        .iter()
        .map(|c| {
            IntegrationId::new(c.clone())
                .map_err(|e| CommandError::new(ErrorCode::InvalidInput, e.to_string()))
        })
        .collect::<Result<_, _>>()?;
    let transports = &definition.capabilities.mcp_transports;
    if let Err(reason) = adapter::mcp_support(definition.id.as_str(), transports) {
        if chosen.is_empty() {
            return Ok(McpSelection {
                attached: Vec::new(),
                prepared: Vec::new(),
                skipped: Vec::new(),
            });
        }
        return Err(configure_error(ConfigureError::McpUnsupported {
            agent: definition.name.clone(),
            reason,
        }));
    }
    let servers = mcp.servers();
    let selected = x8ai_mcp::attach(&servers, root, &chosen)
        .map_err(|e| CommandError::new(ErrorCode::InvalidInput, e.to_string()))?;
    let mut attached = Vec::new();
    for server in selected {
        if transports.contains(&server.transport.kind()) {
            attached.push(server.clone());
        } else if chosen.contains(&server.id) {
            return Err(CommandError::new(
                ErrorCode::InvalidInput,
                format!(
                    "{} cannot use {}: it does not support that transport",
                    definition.name, server.name
                ),
            ));
        }
    }
    let (prepared, skipped) = prepare_all(mcp, &attached, path);
    Ok(McpSelection {
        attached: attached.iter().map(|s| s.id.clone()).collect(),
        prepared,
        skipped,
    })
}

/// The servers another run of a session uses: those it was created with that
/// still exist, are enabled, and can run. Never one it did not have.
fn mcp_for_run(
    mcp: &Mcp,
    definition: &AgentDefinition,
    root: &Path,
    attached: &[IntegrationId],
    path: Option<&str>,
) -> McpSelection {
    let servers = mcp.servers();
    let (kept, mut skipped) = x8ai_mcp::still_attached(&servers, root, attached);
    let transports = &definition.capabilities.mcp_transports;
    let supported = adapter::mcp_support(definition.id.as_str(), transports);
    let mut usable = Vec::new();
    for server in kept {
        match &supported {
            Err(reason) => skipped.push((server.id.clone(), reason.clone())),
            Ok(()) if !transports.contains(&server.transport.kind()) => {
                skipped.push((
                    server.id.clone(),
                    "the agent does not support its transport".to_owned(),
                ));
            }
            Ok(()) => usable.push(server.clone()),
        }
    }
    let (prepared, more) = prepare_all(mcp, &usable, path);
    skipped.extend(more);
    McpSelection {
        attached: attached.to_vec(),
        prepared,
        skipped,
    }
}

/// Each server as it would run; one whose command is not found or whose secret
/// is not saved is left out, with the reason. Nothing half configured runs.
fn prepare_all(
    mcp: &Mcp,
    servers: &[x8ai_core::mcp::McpServer],
    path: Option<&str>,
) -> (Vec<Prepared>, Vec<(IntegrationId, String)>) {
    let mut prepared = Vec::new();
    let mut skipped = Vec::new();
    for server in servers {
        let missing: Vec<&str> = server
            .secret_names()
            .filter(|name| {
                !mcp.secrets
                    .contains(&x8ai_mcp::secret_account(server.id.as_str(), name))
                    .unwrap_or(false)
            })
            .collect();
        if !missing.is_empty() {
            skipped.push((
                server.id.clone(),
                format!("secret not saved: {}", missing.join(", ")),
            ));
            continue;
        }
        match x8ai_mcp::prepare(server, path) {
            Ok(p) => prepared.push(p),
            Err(error) => skipped.push((server.id.clone(), error.to_string())),
        }
    }
    (prepared, skipped)
}

/// What the approval dialog says about MCP servers: each one's transport, the
/// exact command or URL, and its variables by name.
fn describe_servers(agent: &str, servers: &[&Prepared]) -> String {
    let mut text = String::new();
    for prepared in servers {
        let server = &prepared.server;
        match &prepared.material.transport {
            MaterialTransport::Stdio { program, args } => {
                let command = std::iter::once(program.display().to_string())
                    .chain(args.iter().map(|a| {
                        if a.contains(char::is_whitespace) || a.is_empty() {
                            format!("“{a}”")
                        } else {
                            a.clone()
                        }
                    }))
                    .collect::<Vec<_>>()
                    .join(" ");
                text.push_str(&format!(
                    "• {} (stdio): {command}
",
                    server.name
                ));
                let variables: Vec<String> = server
                    .env
                    .iter()
                    .map(|v| match v.source {
                        McpEnvSource::Secret => format!("{} (saved secret)", v.name),
                        McpEnvSource::Inherit => format!("{} (from your shell)", v.name),
                    })
                    .collect();
                if !variables.is_empty() {
                    text.push_str(&format!(
                        "   Variables: {}
",
                        variables.join(", ")
                    ));
                }
                text.push_str(&format!(
                    "   Started by the app when {agent} connects, with no other variables of yours.
"
                ));
            }
            MaterialTransport::StreamableHttp { url } => {
                text.push_str(&format!(
                    "• {} (HTTP): {url}
   {agent} connects to it directly.
",
                    server.name
                ));
            }
        }
    }
    text
}

/// Asks the user, in a native dialog, to allow what is not yet allowed: the
/// agent's launch (if `agent_needed`), and the MCP servers `servers`. Records
/// the approvals if the user agrees. Returns whether they did.
fn ask_approval(
    window: &WebviewWindow,
    app: &AppHandle,
    plan: &LaunchPlan,
    agent_needed: bool,
    servers: &[&Prepared],
    skills: &[String],
) -> Result<bool, CommandError> {
    let workspaces = app.state::<Workspaces>();
    let root = plan.workspace.clone();
    let folder = root.file_name().map_or_else(
        || root.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let skills_text = if skills.is_empty() {
        String::new()
    } else {
        format!(
            "Skills for this session: {} (instructions only: they run nothing and need no approval)\n\n",
            skills.join(", ")
        )
    };
    let mcp_text = if servers.is_empty() {
        skills_text
    } else {
        format!(
            "MCP servers for {name}'s sessions here:\n{}\n{skills_text}",
            describe_servers(&plan.name, servers),
            name = plan.name
        )
    };
    let (title, message) = if agent_needed {
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
        (
            format!("Allow {} to work in “{folder}”?", plan.name),
            format!(
                "{name} will work in:\n{root}\n(in a Git worktree of its own for each session, \
                 when the folder is a Git repository)\n\nProgram: {command}\n{model_line}{mcp_text}{name} \
                 runs as you, with access to your files, network and credentials, as if you started \
                 it in a terminal yourself. This allows it in this folder only. You can revoke it in \
                 the Agents panel.",
                name = plan.name,
                root = root.display(),
            ),
        )
    } else {
        let what = match servers {
            [one] => format!("the MCP server “{}”", one.server.name),
            many => format!("{} MCP servers", many.len()),
        };
        (
            format!("Allow {what} in “{folder}”?"),
            format!(
                "Workspace: {root}\n\n{mcp_text}MCP servers run as you, with access to your files and \
                 network. This allows exactly this configuration in this folder only: a change to a \
                 server's command, arguments, URL or variables asks again.",
                root = root.display(),
            ),
        )
    };
    let confirmed = window
        .dialog()
        .message(message)
        .title(title)
        .kind(MessageDialogKind::Warning)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Allow".into(),
            "Cancel".into(),
        ))
        .parent(window)
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
    if agent_needed {
        workspaces.approve(plan)?;
    }
    if !servers.is_empty() {
        let pairs: Vec<(&str, &x8ai_mcp::Material)> = servers
            .iter()
            .map(|p| (p.server.id.as_str(), &p.material))
            .collect();
        app.state::<Mcp>()
            .with_approvals(|a| a.approve(&root, &pairs))?
            .map_err(crate::mcp::io_error)?;
    }
    Ok(true)
}

/// Whether the MCP servers of `selection` may run in `root`, for a clear error
/// before anything starts.
fn check_mcp(app: &AppHandle, root: &Path, selection: &McpSelection) -> Result<(), CommandError> {
    let mcp = app.state::<Mcp>();
    let outcome = app
        .state::<Workspaces>()
        .with_trust(|trust| {
            mcp.with_approvals(|approvals| {
                x8ai_mcp::authorize(root, &selection.prepared, trust, approvals).map(|_| ())
            })
        })
        .ok_or_else(|| CommandError::new(ErrorCode::PermissionDenied, "trust is unavailable"))??;
    outcome.map_err(mcp_denied)
}

fn mcp_denied(reason: x8ai_mcp::Denied) -> CommandError {
    CommandError::new(ErrorCode::PermissionDenied, format!("MCP: {reason}"))
}

/// The login environment, as the runtime reads it (once, kept).
pub(crate) async fn login_environment(app: &AppHandle) -> Result<Arc<Resolved>, CommandError> {
    resolved(app, false).await
}

/// The skills a new session of `definition` in `root` gets: global and
/// workspace skills, and the session skills `chosen`. For an agent that cannot
/// take skills, choosing one is refused and the others are not attached.
fn skills_for_new_session(
    skills: &Skills,
    definition: &AgentDefinition,
    root: &Path,
    chosen: &[String],
) -> Result<Vec<Skill>, CommandError> {
    let chosen: Vec<IntegrationId> = chosen
        .iter()
        .map(|c| {
            IntegrationId::new(c.clone())
                .map_err(|e| CommandError::new(ErrorCode::InvalidInput, e.to_string()))
        })
        .collect::<Result<_, _>>()?;
    if let Err(reason) = adapter::skills_support(definition.id.as_str()) {
        if chosen.is_empty() {
            return Ok(Vec::new());
        }
        return Err(configure_error(ConfigureError::SkillsUnsupported {
            agent: definition.name.clone(),
            reason,
        }));
    }
    let all = skills.skills();
    let attached = x8ai_skills::attach(&all, root, &chosen)
        .map_err(|e| CommandError::new(ErrorCode::InvalidInput, e.to_string()))?;
    Ok(attached.into_iter().cloned().collect())
}

/// The skills exactly as a session recorded them, for the agent; refused, with
/// the reason, when one was removed or changed since.
fn skills_for_run(skills: &Skills, recorded: &[SkillRef]) -> Result<Vec<AgentSkill>, CommandError> {
    let all = skills.skills();
    let resolved = x8ai_skills::resolve(&all, recorded)
        .map_err(|e| CommandError::new(ErrorCode::Conflict, format!("Skills: {e}")))?;
    Ok(resolved
        .into_iter()
        .map(|s| AgentSkill {
            reference: s.reference(),
            name: s.name.clone(),
            instructions: s.instructions.clone(),
        })
        .collect())
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

/// The built-in agent definitions.
pub(crate) fn definitions(app: &AppHandle) -> Vec<AgentDefinition> {
    app.state::<Agents>().definitions.clone()
}

pub(crate) fn home() -> PathBuf {
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
    let statuses = agent_statuses(&agents, &environment, &workspaces, &providers);
    let isolation = root
        .as_ref()
        .map(|root| isolation_of(git_in(&environment).as_ref(), root));
    Ok(AgentList {
        agents: statuses,
        environment_problem: environment.problem.clone(),
        isolation,
    })
}

/// Every built-in agent's status, as the runtime reports it (for the Agents view
/// and the catalog). Finding a program runs nothing.
pub(crate) fn agent_statuses(
    agents: &Agents,
    environment: &Resolved,
    workspaces: &Workspaces,
    providers: &Providers,
) -> Vec<AgentStatus> {
    let root = workspaces.root();
    // Without a workspace, agents are still looked up; they are just not approved.
    let lookup_in = root.clone().unwrap_or_else(|| PathBuf::from("/"));
    agents
        .definitions
        .iter()
        .map(|definition| {
            x8ai_agents::status(
                definition,
                &environment.vars,
                &lookup_in,
                providers.definitions(),
                |plan| root.is_some() && workspaces.is_approved_for_any_provider(plan),
            )
        })
        .collect()
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
/// or pointed at `model`, and with the MCP servers a new session would get (the
/// session servers `mcp` among them): the workspace must be trusted, and what is
/// not yet approved there (the executable it would run now, the provider and
/// endpoint `model` would use, each MCP server exactly as it would run) is asked
/// for in one native dialog. Returns whether all is approved now; `false` if the
/// user declined.
#[tauri::command]
pub async fn agent_request_approval(
    agent: String,
    model: Option<ModelSelection>,
    mcp: Option<Vec<String>>,
    skills: Option<Vec<String>>,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<bool, CommandError> {
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
        let selection = mcp_for_new_session(
            &task_app.state::<Mcp>(),
            definition,
            &root,
            &mcp.unwrap_or_default(),
            var(&environment.vars, "PATH"),
        )?;
        let skills = skills_for_new_session(
            &task_app.state::<Skills>(),
            definition,
            &root,
            &skills.unwrap_or_default(),
        )?;
        let names: Vec<String> = skills.iter().map(|s| s.name.clone()).collect();
        request_approval(&window, &task_app, &plan, &selection, &names)
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

/// The same for an existing session, before it runs again: what it would run
/// now (its model, the MCP servers it still has) must be approved. Asks only for
/// what is not.
#[tauri::command]
pub async fn agent_request_session_approval(
    session: AgentSessionId,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<bool, CommandError> {
    let environment = resolved(&app, false).await?;
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let agents = task_app.state::<Agents>();
        let workspaces = task_app.state::<Workspaces>();
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
        let plan = launch_plan(
            definition,
            &environment,
            &root,
            record.model.as_ref(),
            &task_app.state::<Providers>(),
        )?;
        let selection = mcp_for_run(
            &task_app.state::<Mcp>(),
            definition,
            &root,
            &record.mcp,
            var(&environment.vars, "PATH"),
        );
        let names: Vec<String> = task_app
            .state::<Skills>()
            .session_skills(&record.skills)
            .into_iter()
            .map(|s| s.name)
            .collect();
        request_approval(&window, &task_app, &plan, &selection, &names)
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

fn request_approval(
    window: &WebviewWindow,
    app: &AppHandle,
    plan: &LaunchPlan,
    selection: &McpSelection,
    skills: &[String],
) -> Result<bool, CommandError> {
    let workspaces = app.state::<Workspaces>();
    if !workspaces.is_trusted(&plan.workspace) {
        return Err(denied(Denied::Untrusted(plan.workspace.clone())));
    }
    let agent_needed = !workspaces.is_approved(plan);
    let servers: Vec<&Prepared> = app
        .state::<Mcp>()
        .with_approvals(|a| x8ai_mcp::unapproved(&plan.workspace, &selection.prepared, a))?;
    if !agent_needed && servers.is_empty() {
        return Ok(true);
    }
    ask_approval(window, app, plan, agent_needed, &servers, skills)
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
/// configuration or pointed at `model`, and the MCP servers a new session gets
/// (the session servers `mcp` among them): in a Git repository, a new worktree on
/// a new branch from the checked-out commit; otherwise the folder itself, for one
/// agent at a time. Refused unless the workspace is trusted and the agent and its
/// MCP servers approved there. Nothing runs yet. The user's working tree is not
/// touched.
#[tauri::command]
pub async fn agent_create_session(
    agent: String,
    model: Option<ModelSelection>,
    mcp: Option<Vec<String>>,
    skills: Option<Vec<String>>,
    app: AppHandle,
) -> Result<AgentSessionInfo, CommandError> {
    let environment = resolved(&app, false).await?;
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let agents = task_app.state::<Agents>();
        let workspaces = task_app.state::<Workspaces>();
        let definition = agents.definition(&agent)?;
        let root = open_root(&workspaces)?;
        let mut plan = launch_plan(
            definition,
            &environment,
            &root,
            model.as_ref(),
            &task_app.state::<Providers>(),
        )?;
        let selection = mcp_for_new_session(
            &task_app.state::<Mcp>(),
            definition,
            &root,
            &mcp.unwrap_or_default(),
            var(&environment.vars, "PATH"),
        )?;
        // 1. trust, 2. approval, of the agent and its MCP servers.
        workspaces.authorize(&plan).map_err(denied)?;
        check_mcp(&task_app, &root, &selection)?;
        plan.mcp = selection.attached;
        // Skills are recorded exactly as they are now.
        plan.skills = skills_for_new_session(
            &task_app.state::<Skills>(),
            definition,
            &root,
            &skills.unwrap_or_default(),
        )?
        .iter()
        .map(Skill::reference)
        .collect();
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
                    .create(
                        git,
                        &repo,
                        &definition.id,
                        plan.model.as_ref(),
                        &plan.mcp,
                        &plan.skills,
                    )
                    .map_err(isolation_error)?;
                (cwd_in(&worktree.path, &repo), Some(worktree))
            }
            _ => (root.clone(), None),
        };
        let id = agents
            .runtime
            .create(&plan, cwd, worktree)
            .map_err(run_error)?;
        session_info(
            &agents.runtime.get(id).expect("just created"),
            &task_app.state::<Mcp>(),
            &task_app.state::<Skills>(),
        )
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

/// Runs the agent of `session` (again, after it stopped) on a new terminal
/// session, in the session's own directory, with the session's model. Refused
/// unless the session belongs to the open workspace, the workspace is trusted,
/// the agent approved there for the session's provider, and the provider's key
/// is still saved. Its MCP servers are the ones it was created with that are
/// still enabled, each approved as it would run; stdio servers get sockets and
/// start when the agent connects, and stop when it ends. Output and exit arrive
/// on `events` as for `terminal_create`.
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
        let fail = |error: CommandError| {
            agents.runtime.fail(session, error.message.clone());
            error
        };
        workspaces
            .authorize(&plan)
            .map_err(|reason| fail(denied(reason)))?;

        // Its MCP servers, checked the same way, then ready for the agent.
        let mcp = task_app.state::<Mcp>();
        let selection = mcp_for_run(
            &mcp,
            definition,
            &root,
            &record.mcp,
            var(&environment.vars, "PATH"),
        );
        check_mcp(&task_app, &root, &selection).map_err(fail)?;
        // Its skills exactly as recorded: a removed or changed one stops it here.
        let session_skills =
            skills_for_run(&task_app.state::<Skills>(), &record.skills).map_err(fail)?;
        let plan = start_mcp(
            &task_app,
            session,
            &environment,
            definition,
            plan,
            &record.cwd,
            &selection,
        )
        .map_err(fail)?;
        let plan = adapter::attach_skills(plan, &session_skills).map_err(|error| {
            mcp.stop(session.0);
            fail(configure_error(error))
        })?;
        let authorized = workspaces.authorize(&plan).map_err(|reason| {
            mcp.stop(session.0);
            fail(denied(reason))
        })?;
        let events = Arc::new(AgentEvents {
            channel: ChannelEvents(events),
            app: task_app.clone(),
            session: session.0,
            mcp_run: mcp.runtime.run_token(session.0),
        });
        let pty = agents
            .runtime
            .run(terminals.sessions(), session, authorized, size, events)
            .map_err(|error| {
                mcp.stop(session.0);
                run_error(error)
            })?;
        if let Some(pid) = pty.pid() {
            mcp.runtime.set_owner(session.0, pid);
        }
        Ok(info(&pty))
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

/// Prepares the MCP servers of a run: a socket for each stdio server (started
/// when the agent connects), and the agent configured, through its adapter, to
/// reach them. Returns the plan with the MCP configuration added.
fn start_mcp(
    app: &AppHandle,
    session: AgentSessionId,
    environment: &Resolved,
    definition: &AgentDefinition,
    plan: LaunchPlan,
    cwd: &Path,
    selection: &McpSelection,
) -> Result<LaunchPlan, CommandError> {
    let mcp = app.state::<Mcp>();
    // An earlier run of this session is stopped first, without holding any store.
    mcp.stop(session.0);
    let mut launches = Vec::new();
    for prepared in &selection.prepared {
        let env = x8ai_mcp::environment(&prepared.server, &environment.vars, mcp.secrets.as_ref())
            .map_err(|e| CommandError::new(ErrorCode::NotFound, format!("MCP: {e}")))?;
        launches.extend(Launch::new(prepared, env, cwd.to_owned()));
    }
    let endpoints = if launches.is_empty() {
        Vec::new()
    } else {
        let root = plan.workspace.clone();
        app.state::<Workspaces>()
            .with_trust(|trust| {
                mcp.with_approvals(|approvals| {
                    let authorized =
                        x8ai_mcp::authorize(&root, &selection.prepared, trust, approvals)
                            .map_err(mcp_denied)?;
                    mcp.runtime
                        .start(session.0, &authorized, launches)
                        .map_err(|e| CommandError::new(ErrorCode::Internal, format!("MCP: {e}")))
                })
            })
            .ok_or_else(|| {
                CommandError::new(ErrorCode::PermissionDenied, "trust is unavailable")
            })???
    };
    let bridge = crate::mcp::bridge()?;
    let servers: Vec<AgentMcpServer> = selection
        .prepared
        .iter()
        .map(|prepared| AgentMcpServer {
            id: prepared.server.id.clone(),
            transport: match &prepared.server.transport {
                McpServerTransport::StreamableHttp { url } => {
                    AgentMcpTransport::StreamableHttp { url: url.clone() }
                }
                McpServerTransport::Stdio { .. } => {
                    let socket = endpoints
                        .iter()
                        .find(|e| e.id == prepared.server.id)
                        .map(|e| e.socket.display().to_string())
                        .unwrap_or_default();
                    AgentMcpTransport::Stdio {
                        command: bridge.clone(),
                        args: vec![x8ai_mcp::bridge::FLAG.to_owned(), socket],
                    }
                }
            },
        })
        .collect();
    mcp.record_run(
        session.0,
        selection
            .attached
            .iter()
            .map(|id| RunServer {
                id: id.clone(),
                transport: selection
                    .prepared
                    .iter()
                    .find(|p| p.server.id == *id)
                    .map(|p| p.server.transport.kind()),
                skipped: selection
                    .skipped
                    .iter()
                    .find(|(s, _)| s == id)
                    .map(|(_, r)| r.clone()),
            })
            .collect(),
    );
    adapter::attach_mcp(plan, &definition.capabilities.mcp_transports, &servers).map_err(|e| {
        mcp.stop(session.0);
        configure_error(e)
    })
}

/// A running agent's terminal events, and the end of its MCP servers when it
/// exits.
struct AgentEvents {
    channel: ChannelEvents,
    app: AppHandle,
    session: u32,
    /// The run of MCP servers started with this agent.
    mcp_run: Option<u64>,
}

impl SessionEvents for AgentEvents {
    fn output(&self, bytes: Vec<u8>) {
        self.channel.output(bytes);
    }

    fn error(&self, message: String) {
        self.channel.error(message);
    }

    fn exited(&self, exit: TerminalExit) {
        self.channel.exited(exit);
        // Off the terminal's thread: stopping waits for the servers to exit. Only
        // this agent's run: a restart may already have started another.
        if let Some(token) = self.mcp_run {
            let (app, session) = (self.app.clone(), self.session);
            std::thread::spawn(move || app.state::<Mcp>().runtime.stop_run(session, token));
        }
    }
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
            .map(|s| session_info(s, &task_app.state::<Mcp>(), &task_app.state::<Skills>()))
            .collect()
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

/// Stops the agent of `session`. Its worktree stays. Its MCP servers stop when
/// it has exited.
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

fn session_info(
    session: &AgentSession,
    mcp: &Mcp,
    skills: &Skills,
) -> Result<AgentSessionInfo, CommandError> {
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
        mcp: mcp.session_servers(
            session.id.0,
            &session.mcp,
            session.state == SessionState::Running,
        ),
        skills: skills.session_skills(&session.skills),
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
        | ConfigureError::InvalidModel(_)
        | ConfigureError::McpUnsupported { .. }
        | ConfigureError::SkillsUnsupported { .. }
        | ConfigureError::SkillsTooLong => ErrorCode::InvalidInput,
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
