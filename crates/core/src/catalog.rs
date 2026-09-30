//! The catalog (Phase 8, docs/catalog.md, ADR 0018): one place to discover the
//! agents, models, MCP servers and skills the app knows, and their state. It is
//! presentation over the systems that own them: an item carries the owning
//! system's id and the state that system reports, never a configuration of its
//! own, and nothing in it can run, approve or unlock anything.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::id::IntegrationId;
use crate::mcp::{McpScope, McpTransportKind};
use crate::model::{CredentialState, LocalAvailability, ModelSource, ProviderKind};
use crate::skill::{SkillScope, SkillSource};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CatalogItemType {
    Agent,
    /// A model, or the provider that serves models.
    Model,
    McpServer,
    Skill,
}

/// Where an item's metadata comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CatalogSource {
    /// Shipped with the app.
    Builtin,
    /// Found on this machine (a model Ollama has).
    Local,
    /// Added by the user (an MCP server, a model id, a skill).
    UserDefined,
    /// A signed remote catalog. Not implemented: no item has this source, and
    /// metadata claiming it is refused.
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CatalogStatus {
    /// On this machine, as its owning system reports (an agent's program found,
    /// a skill in the app, a local server installed).
    Installed,
    /// Can be used once set up (a provider without a key, an MCP server missing
    /// a secret).
    Available,
    /// Set up and ready.
    Configured,
    /// The app cannot use it here (no agent supports it, another platform).
    Unsupported,
    /// Not present or turned off (a program not installed, a disabled server).
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogItem {
    /// `agent.claude-code`, `provider.anthropic`, `model.anthropic.claude-sonnet-5`,
    /// `mcp.github`, `skill.python-debugging`.
    pub id: String,
    #[serde(rename = "type")]
    #[ts(rename = "type")]
    pub item_type: CatalogItemType,
    /// The owning system's name for it.
    pub name: String,
    pub display_name: String,
    pub description: String,
    /// The version of the catalog's metadata for it, not of any software.
    pub catalog_version: Option<String>,
    /// The installed software's version, when known. The app does not run a
    /// program to find out, so it is empty for agents.
    pub software_version: Option<String>,
    /// Only when known.
    pub publisher: Option<String>,
    pub source: CatalogSource,
    /// Only what the owning system knows it can do.
    pub capabilities: Vec<String>,
    pub tags: Vec<String>,
    pub status: CatalogStatus,
    /// Why, in a few words (`Not installed: codex was not found on your PATH`).
    pub status_detail: Option<String>,
    /// What it needs before it can be used.
    pub requirements: Vec<String>,
    pub details: CatalogDetails,
}

/// What each kind of item links to in its owning system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum CatalogDetails {
    Agent {
        agent: IntegrationId,
        /// Where the runtime found its program.
        executable: Option<String>,
        /// Providers the app can point it at.
        providers: Vec<IntegrationId>,
        mcp: bool,
        skills: bool,
    },
    Provider {
        provider: IntegrationId,
        hosting: ProviderKind,
        credential: CredentialState,
        local: Option<LocalAvailability>,
    },
    Model {
        provider: IntegrationId,
        provider_name: String,
        model: String,
        source: ModelSource,
    },
    McpServer {
        server: IntegrationId,
        transport: McpTransportKind,
        enabled: bool,
        scope: McpScope,
    },
    Skill {
        skill: IntegrationId,
        version: u32,
        source: SkillSource,
        scope: SkillScope,
        allowed_tools: Vec<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CatalogList {
    pub items: Vec<CatalogItem>,
    /// Metadata that could not be used, and why.
    pub warnings: Vec<String>,
}
