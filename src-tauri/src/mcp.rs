//! MCP commands: the IPC face of `x8ai-mcp` (docs/mcp.md).
//!
//! The webview can list the MCP servers the user configured, add, change, enable,
//! disable and remove them, and save or delete a server's secret variables. It
//! can never read a secret back, and never start a server: stdio servers are
//! started by the agent commands, for an agent session, once the workspace is
//! trusted and the server approved there. Nothing here contacts a server.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Manager};
use x8ai_agents::adapter;
use x8ai_core::agent::AgentDefinition;
use x8ai_core::error::{CommandError, ErrorCode};
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{
    McpAgentSupport, McpEnvSource, McpSecretStatus, McpServer, McpServerInput, McpServerList,
    McpServerState, McpServerStatus, McpTransportKind, SessionMcpServer,
};
use x8ai_core::model::CredentialState;
use x8ai_mcp::environment::{KEYCHAIN_LABEL, KEYCHAIN_SERVICE};
use x8ai_mcp::{Approvals, Limits, McpRuntime, Registry, prepare, secret_account};
use x8ai_secrets::{Keychain, SecretStore, SecretValue};

use crate::workspace::Workspaces;

/// The registry, the approvals, the Keychain, and the servers of running
/// sessions. Managed Tauri state.
pub struct Mcp {
    registry: Mutex<Option<Registry>>,
    approvals: Mutex<Option<Approvals>>,
    pub(crate) secrets: Box<dyn SecretStore>,
    pub(crate) runtime: McpRuntime,
    /// What each session's last run did with its servers.
    runs: Mutex<HashMap<u32, Vec<RunServer>>>,
}

/// One of a session's servers, as its last run used it.
#[derive(Debug, Clone)]
pub(crate) struct RunServer {
    pub id: IntegrationId,
    pub transport: Option<McpTransportKind>,
    /// Why it was left out of the run, if it was.
    pub skipped: Option<String>,
}

impl Default for Mcp {
    fn default() -> Self {
        Self::new(
            Box::new(Keychain::new(KEYCHAIN_SERVICE, KEYCHAIN_LABEL)),
            crate::agents::home().join(".x8ai/mcp"),
        )
    }
}

impl Mcp {
    fn new(secrets: Box<dyn SecretStore>, sockets: PathBuf) -> Self {
        Self {
            registry: Mutex::new(None),
            approvals: Mutex::new(None),
            secrets,
            runtime: McpRuntime::new(sockets, Limits::default()),
            runs: Mutex::new(HashMap::new()),
        }
    }
}

impl Mcp {
    /// Loads the registry and approvals from the app's data directory, and
    /// removes sockets a crashed run left behind. Starts nothing.
    pub fn load(&self, data_dir: &Path, workspaces: &Workspaces) {
        let (registry, warnings) = Registry::load(data_dir.join("mcp-servers.json"));
        for warning in warnings {
            workspaces.warn(warning);
        }
        let (approvals, warning) = Approvals::load(data_dir.join("mcp-approvals.json"));
        if let Some(warning) = warning {
            workspaces.warn(warning);
        }
        *lock(&self.registry) = Some(registry);
        *lock(&self.approvals) = Some(approvals);
        self.runtime.sweep();
    }

    /// The registered servers, as they are now.
    pub(crate) fn servers(&self) -> Vec<McpServer> {
        lock(&self.registry)
            .as_ref()
            .map(|r| r.servers().to_vec())
            .unwrap_or_default()
    }

    pub(crate) fn with_approvals<T>(
        &self,
        f: impl FnOnce(&mut Approvals) -> T,
    ) -> Result<T, CommandError> {
        let mut approvals = lock(&self.approvals);
        let approvals = approvals.as_mut().ok_or_else(|| {
            CommandError::new(ErrorCode::Internal, "MCP approvals are unavailable")
        })?;
        Ok(f(approvals))
    }

    fn with_registry<T>(
        &self,
        f: impl FnOnce(&mut Registry) -> Result<T, x8ai_mcp::registry::Error>,
    ) -> Result<T, CommandError> {
        let mut registry = lock(&self.registry);
        let registry = registry.as_mut().ok_or_else(|| {
            CommandError::new(ErrorCode::Internal, "the MCP registry is unavailable")
        })?;
        f(registry).map_err(registry_error)
    }

    /// Records what a session's run did with its servers.
    pub(crate) fn record_run(&self, session: u32, servers: Vec<RunServer>) {
        lock(&self.runs).insert(session, servers);
    }

    /// Stops the servers of `session`; called when its agent ends.
    pub(crate) fn stop(&self, session: u32) {
        self.runtime.stop(session);
    }

    pub(crate) fn stop_all(&self) {
        self.runtime.stop_all();
    }

    /// Forgets MCP approvals in `root`, as when its trust is removed.
    pub(crate) fn revoke_all(&self, root: &Path) -> Result<(), CommandError> {
        self.with_approvals(|a| a.revoke_all(root))?
            .map_err(io_error)
    }

    /// The servers attached to a session, and what each is doing. `running` is
    /// whether its agent runs.
    pub(crate) fn session_servers(
        &self,
        session: u32,
        attached: &[IntegrationId],
        running: bool,
    ) -> Vec<SessionMcpServer> {
        let servers = self.servers();
        let states = self.runtime.states(session);
        let runs = lock(&self.runs);
        let last = runs.get(&session);
        attached
            .iter()
            .map(|id| {
                let server = servers.iter().find(|s| s.id == *id);
                let run = last.and_then(|r| r.iter().find(|s| s.id == *id));
                let transport = run
                    .and_then(|r| r.transport)
                    .or_else(|| server.map(|s| s.transport.kind()));
                let state = match (
                    run.and_then(|r| r.skipped.clone()),
                    states.iter().find(|(s, _)| s == id),
                ) {
                    (Some(reason), _) => McpServerState::Skipped { reason },
                    (None, Some((_, state))) => state.clone(),
                    (None, None)
                        if running && transport == Some(McpTransportKind::StreamableHttp) =>
                    {
                        McpServerState::Remote
                    }
                    (None, None) => McpServerState::Idle,
                };
                SessionMcpServer {
                    id: id.clone(),
                    name: server.map_or_else(|| id.to_string(), |s| s.name.clone()),
                    transport,
                    state,
                }
            })
            .collect()
    }

    fn status(
        &self,
        server: &McpServer,
        path: Option<&str>,
        agents: &[AgentDefinition],
    ) -> McpServerStatus {
        let secrets: Vec<McpSecretStatus> = server
            .secret_names()
            .map(|name| McpSecretStatus {
                name: name.to_owned(),
                state: match self
                    .secrets
                    .contains(&secret_account(server.id.as_str(), name))
                {
                    Ok(true) => CredentialState::InKeychain,
                    Ok(false) | Err(_) => CredentialState::Missing,
                },
            })
            .collect();
        let problem = prepare(server, path).err().map(|e| e.to_string());
        let missing: Vec<&str> = secrets
            .iter()
            .filter(|s| s.state == CredentialState::Missing)
            .map(|s| s.name.as_str())
            .collect();
        let problem = problem.or_else(|| {
            (!missing.is_empty()).then(|| format!("secret not saved: {}", missing.join(", ")))
        });
        McpServerStatus {
            configured: problem.is_none(),
            problem,
            secrets,
            agents: agents
                .iter()
                .map(|agent| {
                    let transports = &agent.capabilities.mcp_transports;
                    let support =
                        adapter::mcp_support(agent.id.as_str(), transports).and_then(|()| {
                            if transports.contains(&server.transport.kind()) {
                                Ok(())
                            } else {
                                Err("it does not support this transport".to_owned())
                            }
                        });
                    McpAgentSupport {
                        agent: agent.id.clone(),
                        supported: support.is_ok(),
                        reason: support.err(),
                    }
                })
                .collect(),
            server: server.clone(),
        }
    }
}

/// Every registered server, with its secrets' state (never their values), whether
/// it is ready, and which agents can use it. Checks nothing over the network.
#[tauri::command]
pub async fn mcp_list(app: AppHandle) -> Result<McpServerList, CommandError> {
    let path = crate::agents::login_path(&app).await?;
    blocking(&app, move |app| {
        let mcp = app.state::<Mcp>();
        let agents = crate::agents::definitions(app);
        Ok(McpServerList {
            servers: mcp
                .servers()
                .iter()
                .map(|s| mcp.status(s, path.as_deref(), &agents))
                .collect(),
        })
    })
    .await
}

/// Adds a server. A workspace server belongs to the open folder.
#[tauri::command]
pub async fn mcp_add(
    server: McpServerInput,
    app: AppHandle,
) -> Result<McpServerStatus, CommandError> {
    let path = crate::agents::login_path(&app).await?;
    blocking(&app, move |app| {
        let mcp = app.state::<Mcp>();
        let root = app.state::<Workspaces>().root();
        let added = mcp.with_registry(|r| r.add(&server, root.as_deref()))?;
        Ok(mcp.status(&added, path.as_deref(), &crate::agents::definitions(app)))
    })
    .await
}

/// Changes a server. Its id stays; a secret variable it no longer has is deleted
/// from the Keychain. A change to what it runs needs a new approval.
#[tauri::command]
pub async fn mcp_update(
    id: String,
    server: McpServerInput,
    app: AppHandle,
) -> Result<McpServerStatus, CommandError> {
    let path = crate::agents::login_path(&app).await?;
    blocking(&app, move |app| {
        let mcp = app.state::<Mcp>();
        let root = app.state::<Workspaces>().root();
        let (old, new) = mcp.with_registry(|r| r.update(&id, &server, root.as_deref()))?;
        let kept: Vec<&str> = new.secret_names().collect();
        for name in old.secret_names().filter(|n| !kept.contains(n)) {
            mcp.secrets
                .remove(&secret_account(old.id.as_str(), name))
                .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
        }
        Ok(mcp.status(&new, path.as_deref(), &crate::agents::definitions(app)))
    })
    .await
}

/// Enables or disables a server. A disabled server is not attached to new
/// sessions, and not started for existing ones.
#[tauri::command]
pub async fn mcp_set_enabled(
    id: String,
    enabled: bool,
    app: AppHandle,
) -> Result<McpServerStatus, CommandError> {
    let path = crate::agents::login_path(&app).await?;
    blocking(&app, move |app| {
        let mcp = app.state::<Mcp>();
        let server = mcp.with_registry(|r| r.set_enabled(&id, enabled))?;
        Ok(mcp.status(&server, path.as_deref(), &crate::agents::definitions(app)))
    })
    .await
}

/// Removes a server, its secrets and its approvals.
#[tauri::command]
pub async fn mcp_remove(id: String, app: AppHandle) -> Result<(), CommandError> {
    blocking(&app, move |app| {
        let mcp = app.state::<Mcp>();
        let removed = mcp.with_registry(|r| r.remove(&id))?;
        for name in removed.secret_names() {
            mcp.secrets
                .remove(&secret_account(removed.id.as_str(), name))
                .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
        }
        mcp.with_approvals(|a| a.forget(removed.id.as_str()))?
            .map_err(io_error)
    })
    .await
}

/// Saves the value of one of a server's secret variables in the Keychain. The
/// value is not returned, and not kept anywhere else.
#[tauri::command]
pub async fn mcp_set_secret(
    id: String,
    name: String,
    value: String,
    app: AppHandle,
) -> Result<McpServerStatus, CommandError> {
    let secret = SecretValue::new(&value)
        .map_err(|e| CommandError::new(ErrorCode::InvalidInput, e.to_string()));
    drop(value);
    let secret = secret?;
    let path = crate::agents::login_path(&app).await?;
    blocking(&app, move |app| {
        let mcp = app.state::<Mcp>();
        let server = secret_variable(&mcp, &id, &name)?;
        mcp.secrets
            .set(&secret_account(server.id.as_str(), &name), &secret)
            .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
        Ok(mcp.status(&server, path.as_deref(), &crate::agents::definitions(app)))
    })
    .await
}

/// Deletes the saved value of one of a server's secret variables.
#[tauri::command]
pub async fn mcp_remove_secret(
    id: String,
    name: String,
    app: AppHandle,
) -> Result<McpServerStatus, CommandError> {
    let path = crate::agents::login_path(&app).await?;
    blocking(&app, move |app| {
        let mcp = app.state::<Mcp>();
        let server = secret_variable(&mcp, &id, &name)?;
        mcp.secrets
            .remove(&secret_account(server.id.as_str(), &name))
            .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
        Ok(mcp.status(&server, path.as_deref(), &crate::agents::definitions(app)))
    })
    .await
}

/// The server `id`, if `name` is one of its secret variables.
fn secret_variable(mcp: &Mcp, id: &str, name: &str) -> Result<McpServer, CommandError> {
    let server = mcp
        .servers()
        .into_iter()
        .find(|s| s.id.as_str() == id)
        .ok_or_else(|| CommandError::new(ErrorCode::NotFound, format!("no MCP server {id:?}")))?;
    if !server
        .env
        .iter()
        .any(|v| v.name == name && v.source == McpEnvSource::Secret)
    {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            format!("{} has no secret variable {name:?}", server.name),
        ));
    }
    Ok(server)
}

/// Runs `work` off the IPC runtime: the Keychain can block.
async fn blocking<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(&AppHandle) -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || work(&app))
        .await
        .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

fn registry_error(error: x8ai_mcp::registry::Error) -> CommandError {
    use x8ai_mcp::registry::Error;
    let code = match &error {
        Error::Invalid(_) | Error::NoWorkspace | Error::Full => ErrorCode::InvalidInput,
        Error::NotFound(_) => ErrorCode::NotFound,
        Error::Io { .. } => ErrorCode::Internal,
    };
    CommandError::new(code, error.to_string())
}

pub(crate) fn io_error(error: std::io::Error) -> CommandError {
    CommandError::new(ErrorCode::Internal, error.to_string())
}

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The bridge the agent runs for a stdio server: this app's own executable with
/// `--mcp-bridge`.
pub(crate) fn bridge() -> Result<PathBuf, CommandError> {
    std::env::current_exe().map_err(|e| {
        CommandError::new(
            ErrorCode::Internal,
            format!("cannot find the app itself: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use x8ai_core::mcp::{McpEnvVar, McpScopeKind, McpServerTransport};
    use x8ai_secrets::MemoryStore;

    use super::*;

    const SECRET: &str = "ghp_x8aitestinvalid000000000000000000000";

    #[test]
    fn what_the_webview_receives_never_holds_a_secret() {
        let temp =
            std::env::temp_dir().join(format!("x8ai-mcp-desktop-test-{}", std::process::id()));
        let mcp = Mcp::new(Box::new(MemoryStore::default()), temp.join("sockets"));
        *lock(&mcp.registry) = Some(Registry::load(temp.join("mcp-servers.json")).0);
        let input = McpServerInput {
            name: "GitHub".into(),
            description: String::new(),
            transport: McpServerTransport::Stdio {
                command: "/bin/cat".into(),
                args: Vec::new(),
            },
            env: vec![McpEnvVar {
                name: "GITHUB_PERSONAL_ACCESS_TOKEN".into(),
                source: McpEnvSource::Secret,
            }],
            enabled: true,
            scope: McpScopeKind::Global,
        };
        let server = mcp.with_registry(|r| r.add(&input, None)).unwrap();
        let before = mcp.status(&server, Some("/usr/bin:/bin"), &x8ai_agents::builtin());
        assert!(!before.configured);
        assert_eq!(before.secrets[0].state, CredentialState::Missing);
        mcp.secrets
            .set(
                &secret_account("github", "GITHUB_PERSONAL_ACCESS_TOKEN"),
                &SecretValue::new(SECRET).unwrap(),
            )
            .unwrap();
        let after = mcp.status(&server, Some("/usr/bin:/bin"), &x8ai_agents::builtin());
        assert!(after.configured, "{:?}", after.problem);
        assert_eq!(after.secrets[0].state, CredentialState::InKeychain);
        let json = serde_json::to_string(&McpServerList {
            servers: vec![after],
        })
        .unwrap();
        assert!(!json.contains(SECRET), "{json}");
        let file = std::fs::read_to_string(temp.join("mcp-servers.json")).unwrap();
        assert!(!file.contains(SECRET));
        // Both built-in agents can use it.
        assert!(
            json.contains(r#""agent":"claude-code","supported":true"#),
            "{json}"
        );
        let _ = std::fs::remove_dir_all(temp);
    }
}
