//! MCP servers: GitHub, Playwright, filesystem, databases and others.
//!
//! In the initial design the app is not the MCP client: the agent is, and the app
//! supplies it with server configuration. A local (stdio) MCP server is a program
//! running with the user's privileges, so it is described by the same explicit
//! [`LaunchSpec`] as an agent and is subject to the same approval rules.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::definition::{DefinitionError, check_endpoint_url, check_name};
use crate::id::IntegrationId;
use crate::launch::{LaunchSpec, Requirement};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct McpServerDefinition {
    pub id: IntegrationId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub transport: McpTransport,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub enum McpTransport {
    /// A local process speaking MCP over stdin/stdout.
    Stdio { launch: LaunchSpec },
    /// A remote server speaking MCP Streamable HTTP. Authentication (OAuth, per the
    /// MCP specification) is designed in Phase 7.
    StreamableHttp { url: String },
}

/// The transport kinds, used by agents to declare what they can connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum McpTransportKind {
    Stdio,
    StreamableHttp,
}

impl McpServerDefinition {
    pub fn validate(&self) -> Result<(), DefinitionError> {
        check_name(&self.name)?;
        match &self.transport {
            McpTransport::Stdio { launch } => launch.validate("transport.launch")?,
            // Tool calls carry workspace content, so remote servers get the same
            // transport rule as credentials.
            McpTransport::StreamableHttp { url } => {
                check_endpoint_url("transport.url", url, true)?;
            }
        }
        for (i, requirement) in self.requirements.iter().enumerate() {
            requirement.validate(&format!("requirements[{i}]"))?;
        }
        Ok(())
    }
}
