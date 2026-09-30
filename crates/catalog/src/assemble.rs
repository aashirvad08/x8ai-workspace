//! Catalog items from the facts the owning systems report.

use std::collections::BTreeSet;

use x8ai_core::agent::{AgentAvailability, AgentStatus};
use x8ai_core::catalog::{
    CatalogDetails, CatalogItem, CatalogItemType, CatalogList, CatalogSource, CatalogStatus,
};
use x8ai_core::mcp::{McpServerStatus, McpTransportKind};
use x8ai_core::model::{
    CredentialState, LocalAvailability, ModelDefinition, ModelSource, ProviderApi, ProviderKind,
    ProviderStatus,
};
use x8ai_core::skill::{SkillSource, SkillStatus};

use crate::metadata::Metadata;

/// What each owning system reports, as it reports it. The catalog adds nothing
/// to these facts; it presents them.
#[derive(Debug, Clone, Copy)]
pub struct Facts<'a> {
    /// From the agent runtime: every built-in definition, whether its program is
    /// found, and what its adapter supports.
    pub agents: &'a [AgentStatus],
    /// From the provider registry: providers, key state, local availability as
    /// last checked, and their models.
    pub providers: &'a [ProviderStatus],
    /// From the MCP registry.
    pub mcp: &'a [McpServerStatus],
    /// From the skill registry.
    pub skills: &'a [SkillStatus],
}

/// Every item, in the order agents, providers with their models, MCP servers,
/// skills. Metadata that matches no item is reported and not used.
pub fn assemble(metadata: &Metadata, facts: &Facts<'_>) -> CatalogList {
    let mut items = Vec::new();
    for agent in facts.agents {
        items.push(agent_item(metadata, agent));
    }
    for provider in facts.providers {
        items.push(provider_item(metadata, provider));
        for model in &provider.models {
            items.push(model_item(metadata, provider, model));
        }
    }
    for server in facts.mcp {
        items.push(mcp_item(server));
    }
    for skill in facts.skills {
        items.push(skill_item(skill));
    }

    let mut warnings = Vec::new();
    let mut seen = BTreeSet::new();
    items.retain(|item| {
        let first = seen.insert(item.id.clone());
        if !first {
            warnings.push(format!(
                "{} appears twice; the second was not shown",
                item.id
            ));
        }
        first
    });
    for entry in metadata.entries() {
        if !seen.contains(&entry.id) {
            warnings.push(format!(
                "catalog metadata for {} matches nothing the app has; not shown",
                entry.id
            ));
        }
    }
    CatalogList { items, warnings }
}

/// What the agent runtime says about an agent's program.
pub fn agent_status(agent: &AgentStatus) -> (CatalogStatus, Option<String>) {
    match &agent.availability {
        AgentAvailability::Installed { .. } => (CatalogStatus::Installed, None),
        AgentAvailability::NotInstalled { program } => (
            CatalogStatus::Unavailable,
            Some(format!(
                "Not installed: `{program}` was not found on your PATH. The app does not install \
                 programs; install it yourself, then refresh."
            )),
        ),
        AgentAvailability::Unsupported => (
            CatalogStatus::Unsupported,
            Some("Does not run on this operating system.".to_owned()),
        ),
    }
}

/// What the provider registry says: a key saved, or a local server found. A
/// local server is never looked for here; its last known state is used.
pub fn provider_status(provider: &ProviderStatus) -> (CatalogStatus, Option<String>) {
    if provider.hosting == ProviderKind::Local {
        return match &provider.local {
            Some(LocalAvailability::Available { version }) => (
                CatalogStatus::Configured,
                Some(match version {
                    Some(v) => format!("Running (version {v})"),
                    None => "Running".to_owned(),
                }),
            ),
            Some(LocalAvailability::Installed) => (
                CatalogStatus::Installed,
                Some("Installed, not running".to_owned()),
            ),
            Some(LocalAvailability::Unavailable) => (
                CatalogStatus::Unavailable,
                Some("Not found on this machine".to_owned()),
            ),
            None => (
                CatalogStatus::Available,
                Some("Not checked yet: open Models to look for it".to_owned()),
            ),
        };
    }
    match provider.credential {
        CredentialState::InKeychain => (
            CatalogStatus::Configured,
            Some("API key saved in your Keychain".to_owned()),
        ),
        CredentialState::Missing => (
            CatalogStatus::Available,
            Some("No API key saved".to_owned()),
        ),
        CredentialState::NotNeeded => (CatalogStatus::Configured, None),
    }
}

/// A model is ready when its provider is.
pub fn model_status(provider: &ProviderStatus) -> (CatalogStatus, Option<String>) {
    match provider_status(provider).0 {
        CatalogStatus::Configured => (CatalogStatus::Configured, None),
        _ => (
            CatalogStatus::Available,
            Some(format!("{} is not set up yet", provider.name)),
        ),
    }
}

/// What the MCP registry says. Nothing is started to find out.
pub fn mcp_status(server: &McpServerStatus) -> (CatalogStatus, Option<String>) {
    if !server.server.enabled {
        return (CatalogStatus::Unavailable, Some("Disabled".to_owned()));
    }
    if !server.agents.iter().any(|a| a.supported) {
        return (
            CatalogStatus::Unsupported,
            Some("No agent the app can configure supports it".to_owned()),
        );
    }
    match (&server.problem, server.configured) {
        (_, true) => (CatalogStatus::Configured, None),
        (Some(problem), false) => (
            CatalogStatus::Available,
            Some(format!("Not configured: {problem}")),
        ),
        (None, false) => (CatalogStatus::Available, Some("Not configured".to_owned())),
    }
}

/// A skill is in the app, whether built in or the user's; it is usable if an
/// agent can take it.
pub fn skill_status(skill: &SkillStatus) -> (CatalogStatus, Option<String>) {
    if skill.agents.iter().any(|a| a.supported) {
        (CatalogStatus::Installed, None)
    } else {
        (
            CatalogStatus::Unsupported,
            Some("No agent the app can configure takes skills".to_owned()),
        )
    }
}

fn api_label(api: ProviderApi) -> &'static str {
    match api {
        ProviderApi::AnthropicMessages => "Anthropic Messages API",
        ProviderApi::OpenAiChatCompletions => "OpenAI Chat Completions API",
        ProviderApi::OpenAiResponses => "OpenAI Responses API",
        ProviderApi::Gemini => "Gemini API",
    }
}

fn transport_label(kind: McpTransportKind) -> &'static str {
    match kind {
        McpTransportKind::Stdio => "MCP over stdio",
        McpTransportKind::StreamableHttp => "MCP over Streamable HTTP",
    }
}

fn agent_item(metadata: &Metadata, agent: &AgentStatus) -> CatalogItem {
    let id = format!("agent.{}", agent.id);
    let meta = metadata.get(&id);
    let (status, status_detail) = agent_status(agent);
    let supported: Vec<_> = agent
        .providers
        .iter()
        .filter(|p| p.supported)
        .map(|p| p.provider.clone())
        .collect();
    let mut capabilities: Vec<String> = agent
        .capabilities
        .model_apis
        .iter()
        .map(|a| api_label(*a).to_owned())
        .collect();
    capabilities.extend(
        agent
            .capabilities
            .mcp_transports
            .iter()
            .map(|t| transport_label(*t).to_owned()),
    );
    if !supported.is_empty() {
        capabilities.push("Model chosen in the app".to_owned());
    }
    if agent.mcp.supported {
        capabilities.push("MCP servers from the app".to_owned());
    }
    if agent.skills.supported {
        capabilities.push("Skills from the app".to_owned());
    }
    let (executable, requirements) = match &agent.availability {
        AgentAvailability::Installed { executable } => (Some(executable.clone()), Vec::new()),
        AgentAvailability::NotInstalled { program } => (
            None,
            vec![format!("`{program}` installed and on your PATH")],
        ),
        AgentAvailability::Unsupported => (None, vec!["A supported operating system".to_owned()]),
    };
    CatalogItem {
        id,
        item_type: CatalogItemType::Agent,
        name: agent.name.clone(),
        display_name: agent.name.clone(),
        description: agent.description.clone(),
        catalog_version: meta.map(|m| m.version.clone()),
        software_version: None,
        publisher: meta.and_then(|m| m.publisher.clone()),
        source: CatalogSource::Builtin,
        capabilities,
        tags: meta.map(|m| m.tags.clone()).unwrap_or_default(),
        status,
        status_detail,
        requirements,
        details: CatalogDetails::Agent {
            agent: agent.id.clone(),
            executable,
            providers: supported,
            mcp: agent.mcp.supported,
            skills: agent.skills.supported,
        },
    }
}

fn provider_item(metadata: &Metadata, provider: &ProviderStatus) -> CatalogItem {
    let id = format!("provider.{}", provider.id);
    let meta = metadata.get(&id);
    let (status, status_detail) = provider_status(provider);
    let requirements = match (provider.hosting, provider.credential) {
        (ProviderKind::Local, _) => vec![format!(
            "{} installed and running on this machine (the app installs nothing)",
            provider.name
        )],
        (_, CredentialState::NotNeeded) => Vec::new(),
        _ => vec![format!("A {} API key, saved in Models", provider.name)],
    };
    CatalogItem {
        id,
        item_type: CatalogItemType::Model,
        name: provider.name.clone(),
        display_name: provider.name.clone(),
        description: provider.description.clone(),
        catalog_version: meta.map(|m| m.version.clone()),
        software_version: match &provider.local {
            Some(LocalAvailability::Available { version }) => version.clone(),
            _ => None,
        },
        publisher: meta.and_then(|m| m.publisher.clone()),
        source: CatalogSource::Builtin,
        capabilities: Vec::new(),
        tags: meta.map(|m| m.tags.clone()).unwrap_or_default(),
        status,
        status_detail,
        requirements,
        details: CatalogDetails::Provider {
            provider: provider.id.clone(),
            hosting: provider.hosting,
            credential: provider.credential,
            local: provider.local.clone(),
        },
    }
}

fn model_item(
    metadata: &Metadata,
    provider: &ProviderStatus,
    model: &ModelDefinition,
) -> CatalogItem {
    let provider_meta = metadata.get(&format!("provider.{}", provider.id));
    let (status, status_detail) = model_status(provider);
    let source = match model.source {
        ModelSource::BuiltIn => CatalogSource::Builtin,
        ModelSource::Local => CatalogSource::Local,
        ModelSource::Custom => CatalogSource::UserDefined,
    };
    let description = match model.source {
        ModelSource::BuiltIn => format!("A model {} serves.", provider.name),
        ModelSource::Local => format!("A model {} has on this machine.", provider.name),
        ModelSource::Custom => format!("A {} model id you added.", provider.name),
    };
    CatalogItem {
        id: format!("model.{}.{}", provider.id, model.id),
        item_type: CatalogItemType::Model,
        name: model.name.clone(),
        display_name: model.name.clone(),
        description,
        // Only a built-in model's listing is the catalog's; its version is the
        // provider metadata's.
        catalog_version: (model.source == ModelSource::BuiltIn)
            .then(|| provider_meta.map(|m| m.version.clone()))
            .flatten(),
        software_version: None,
        // The provider publishes its own models; a gateway's are someone else's,
        // and a user's id says nothing about who made it.
        publisher: (model.source == ModelSource::BuiltIn
            && provider.hosting == ProviderKind::Hosted)
            .then(|| provider_meta.and_then(|m| m.publisher.clone()))
            .flatten(),
        source,
        capabilities: model
            .context_window
            .map(|tokens| vec![format!("Context window: {tokens} tokens")])
            .unwrap_or_default(),
        tags: provider_meta.map(|m| m.tags.clone()).unwrap_or_default(),
        status,
        status_detail,
        requirements: Vec::new(),
        details: CatalogDetails::Model {
            provider: provider.id.clone(),
            provider_name: provider.name.clone(),
            model: model.id.clone(),
            source: model.source,
        },
    }
}

fn mcp_item(status: &McpServerStatus) -> CatalogItem {
    let server = &status.server;
    let (state, status_detail) = mcp_status(status);
    let mut requirements: Vec<String> = status
        .secrets
        .iter()
        .map(|s| format!("{} saved as a secret", s.name))
        .collect();
    if let x8ai_core::mcp::McpServerTransport::Stdio { command, .. } = &server.transport {
        requirements.push(format!("`{command}` installed (the app installs nothing)"));
    }
    requirements.push("Allowed in the folder, in the approval dialog, before it runs".to_owned());
    CatalogItem {
        id: format!("mcp.{}", server.id),
        item_type: CatalogItemType::McpServer,
        name: server.name.clone(),
        display_name: server.name.clone(),
        description: server.description.clone(),
        catalog_version: None,
        software_version: None,
        publisher: None,
        source: CatalogSource::UserDefined,
        capabilities: vec![transport_label(server.transport.kind()).to_owned()],
        tags: Vec::new(),
        status: state,
        status_detail,
        requirements,
        details: CatalogDetails::McpServer {
            server: server.id.clone(),
            transport: server.transport.kind(),
            enabled: server.enabled,
            scope: server.scope.clone(),
        },
    }
}

fn skill_item(status: &SkillStatus) -> CatalogItem {
    let skill = &status.skill;
    let (state, status_detail) = skill_status(status);
    CatalogItem {
        id: format!("skill.{}", skill.id),
        item_type: CatalogItemType::Skill,
        name: skill.name.clone(),
        display_name: skill.name.clone(),
        description: skill.description.clone(),
        catalog_version: Some(skill.version.to_string()),
        software_version: None,
        publisher: (skill.source == SkillSource::Builtin).then(|| "x8ai Workspace".to_owned()),
        source: match skill.source {
            SkillSource::Builtin => CatalogSource::Builtin,
            SkillSource::User => CatalogSource::UserDefined,
        },
        capabilities: skill
            .allowed_tools
            .iter()
            .map(|t| format!("Suggests the {t} tool"))
            .collect(),
        tags: Vec::new(),
        status: state,
        status_detail,
        requirements: vec![format!("An agent that takes skills from the app ({})", {
            let agents: Vec<&str> = status
                .agents
                .iter()
                .filter(|a| a.supported)
                .map(|a| a.agent.as_str())
                .collect();
            if agents.is_empty() {
                "none here".to_owned()
            } else {
                agents.join(", ")
            }
        })],
        details: CatalogDetails::Skill {
            skill: skill.id.clone(),
            version: skill.version,
            source: skill.source,
            scope: skill.scope.clone(),
            allowed_tools: skill.allowed_tools.clone(),
        },
    }
}
