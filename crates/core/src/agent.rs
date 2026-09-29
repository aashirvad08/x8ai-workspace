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
//! Phase 5 and Phase 7. It is deliberately not modelled here yet.
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
    /// now. `false` when no workspace is open.
    pub approved: bool,
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
