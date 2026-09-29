//! Coding agents: Claude Code, OpenCode, Codex, Aider and anything compatible.
//!
//! An agent is an external program. The app does not implement one; it starts the
//! agent's CLI inside a terminal session (the Phase 1 session substrate, driven by
//! the Phase 4 agent runtime) and, where the agent supports it, hands it a model
//! provider and MCP servers. Nothing in this module is specific to one agent: an
//! agent is compatible with a provider when they share a [`ProviderApi`], and with an
//! MCP server when they share a [`McpTransportKind`].
//!
//! How configuration is injected into each agent (flags, environment variables,
//! config files) differs per agent and is implemented by per-agent adapters in
//! Phase 6 and Phase 7. It is deliberately not modelled here yet.
//!
//! The second half of this module is the IPC contract of the agent runtime
//! (`crates/agents`, `docs/agent-runtime.md`): what the webview learns about each
//! agent. It never contains an environment or anything read from one.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::definition::{DefinitionError, check_name};
use crate::id::IntegrationId;
use crate::launch::{LaunchSpec, Requirement};
use crate::mcp::McpTransportKind;
use crate::model::ProviderApi;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AgentDefinition {
    pub id: IntegrationId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// How to start the agent's interactive CLI inside a terminal session.
    pub launch: LaunchSpec,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    pub capabilities: AgentCapabilities,
    /// Operating systems the agent runs on. Empty means every platform.
    #[serde(default)]
    pub platforms: Vec<Platform>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Platform {
    Macos,
    Linux,
}

impl Platform {
    /// The platform this build runs on, if it is one an agent can declare.
    pub fn current() -> Option<Self> {
        if cfg!(target_os = "macos") {
            Some(Self::Macos)
        } else if cfg!(target_os = "linux") {
            Some(Self::Linux)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct AgentCapabilities {
    /// Model APIs the agent can be pointed at. Empty means the agent manages its
    /// own model access and the app cannot configure it.
    #[serde(default)]
    pub model_apis: Vec<ProviderApi>,
    /// MCP transports the agent can use as an MCP client. Empty means no MCP support.
    #[serde(default)]
    pub mcp_transports: Vec<McpTransportKind>,
}

impl AgentDefinition {
    /// Whether the agent declares support for the platform this build runs on.
    pub fn supports_this_platform(&self) -> bool {
        self.platforms.is_empty()
            || Platform::current().is_some_and(|p| self.platforms.contains(&p))
    }

    pub fn validate(&self) -> Result<(), DefinitionError> {
        check_name(&self.name)?;
        self.launch.validate("launch")?;
        for (i, requirement) in self.requirements.iter().enumerate() {
            requirement.validate(&format!("requirements[{i}]"))?;
        }
        Ok(())
    }
}

/// What the webview knows about one agent: whether it can run here, and whether it
/// is approved for the open workspace. Returned by `agent_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentStatus {
    pub id: IntegrationId,
    pub name: String,
    pub description: String,
    pub availability: AgentAvailability,
    /// Approved to run in the open workspace, with the executable it would run
    /// now, for its own model configuration or for some provider (each needs
    /// its own approval). `false` when no workspace is open.
    pub approved: bool,
    /// Every provider, and whether the app can point this agent at it, as the
    /// agent's adapter says. An agent without an adapter supports none: it uses
    /// only its own configuration.
    pub providers: Vec<ProviderSupport>,
}

/// Whether an agent's adapter can point it at a provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderSupport {
    pub provider: IntegrationId,
    pub supported: bool,
    /// Why not, when not.
    pub reason: Option<String>,
}

/// Where an agent session's model configuration comes from (docs/models.md,
/// ADR 0015). Names variables, never their values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "source",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum SessionConfiguration {
    /// No provider chosen in the app: the agent uses its own configuration and
    /// whatever the user's shell environment provides.
    Agent {
        /// Provider variables the agent would read that the shell sets.
        shell_variables: Vec<String>,
    },
    /// A provider and model chosen in the app. The app's configuration is the
    /// only provider configuration the agent sees.
    App {
        provider: IntegrationId,
        provider_name: String,
        model: String,
        /// Where the agent's model requests go.
        endpoint: String,
        credential: crate::model::CredentialState,
        /// Provider variables set in the shell that this session replaces.
        overridden_shell_variables: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export)]
pub enum AgentAvailability {
    /// Found: the absolute path that would be started.
    Installed { executable: String },
    /// Not found on the user's `PATH`. Nothing is ever installed automatically.
    NotInstalled { program: String },
    /// The agent does not run on this operating system.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentList {
    pub agents: Vec<AgentStatus>,
    /// Set when the user's login-shell environment could not be read, so agents
    /// were looked up with the app's own `PATH` instead. Says why.
    pub environment_problem: Option<String>,
    /// How agents would run in the open workspace; `None` when none is open.
    pub isolation: Option<WorkspaceIsolation>,
}

/// Whether agents in the open workspace get worktrees of their own
/// (docs/multi-agent.md).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum WorkspaceIsolation {
    /// A Git repository: each agent session gets a worktree on a new branch,
    /// starting from this commit.
    Worktrees {
        branch: Option<String>,
        head: String,
    },
    /// No isolation: one agent at a time runs directly in the folder. Says why.
    Unavailable { reason: String },
}

/// Identifies an agent session (a worktree or a slot in the folder, and the
/// agent runs in it) for the lifetime of the app process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AgentSessionId(pub u32);

/// One agent session: where it works and what it is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentSessionInfo {
    pub id: AgentSessionId,
    pub agent: IntegrationId,
    pub name: String,
    /// The workspace root it belongs to.
    pub workspace: String,
    /// Where the agent runs: in its worktree, or the workspace itself.
    pub cwd: String,
    /// `None` when it runs directly in the workspace, without isolation.
    pub worktree: Option<AgentWorktree>,
    /// When the session was created, in milliseconds since the Unix epoch.
    /// A JSON number, which holds milliseconds exactly for any real date.
    #[ts(type = "number")]
    pub started_at: u64,
    pub state: AgentSessionState,
    /// The terminal session while the agent runs.
    pub terminal: Option<crate::terminal::SessionId>,
    pub configuration: SessionConfiguration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentWorktree {
    pub branch: String,
    /// The commit it started from.
    pub base: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export)]
pub enum AgentSessionState {
    /// Created, stopped, or left from an earlier run of the app.
    NotRunning,
    Running,
    Exited {
        exit: crate::terminal::TerminalExit,
    },
    Failed {
        message: String,
    },
}

/// What an agent changed in its worktree since it started.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentChanges {
    pub branch: Option<String>,
    pub base: String,
    pub head: String,
    /// Commits the agent made.
    pub commits: u32,
    /// Some changes are not committed; removing the worktree would lose them.
    pub uncommitted: bool,
    pub files: Vec<AgentChangedFile>,
    /// A unified diff of everything, committed or not, new files included.
    pub diff: String,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentChangedFile {
    /// Relative to the worktree root, `/`-separated.
    pub path: String,
    pub change: ChangeKind,
    /// The old path of a rename.
    pub from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
    Untracked,
    Other,
}

/// What removing an agent's workspace kept.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AgentRemoval {
    /// The agent's branch, kept because it has commits. `None` if it was deleted.
    pub kept_branch: Option<String>,
    pub commits: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn definition(platforms: Vec<Platform>) -> AgentDefinition {
        AgentDefinition {
            id: IntegrationId::new("fixture-agent").unwrap(),
            name: "Fixture".into(),
            description: String::new(),
            launch: LaunchSpec {
                program: "fixture".into(),
                args: vec![],
                env: vec![],
            },
            requirements: vec![],
            capabilities: AgentCapabilities {
                model_apis: vec![],
                mcp_transports: vec![],
            },
            platforms,
        }
    }

    #[test]
    fn no_platforms_means_every_platform() {
        assert!(definition(vec![]).supports_this_platform());
        let here = Platform::current().unwrap();
        assert!(definition(vec![here]).supports_this_platform());
        let elsewhere = if here == Platform::Macos {
            Platform::Linux
        } else {
            Platform::Macos
        };
        assert!(!definition(vec![elsewhere]).supports_this_platform());
    }

    #[test]
    fn availability_is_tagged_by_state() {
        let json = serde_json::to_value(AgentAvailability::Installed {
            executable: "/usr/local/bin/fixture".into(),
        })
        .unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "state": "installed", "executable": "/usr/local/bin/fixture" })
        );
    }
}
