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
    pub fn validate(&self) -> Result<(), DefinitionError> {
        check_name(&self.name)?;
        self.launch.validate("launch")?;
        for (i, requirement) in self.requirements.iter().enumerate() {
            requirement.validate(&format!("requirements[{i}]"))?;
        }
        Ok(())
    }
}
