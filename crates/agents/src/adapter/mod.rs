//! Agent adapters: how one agent is pointed at a provider and a model
//! (docs/models.md, ADR 0015, ADR 0016).
//!
//! The provider layer (`x8ai-providers`) knows providers and models and nothing
//! about agents. An adapter knows one agent: which providers it can use and
//! through which of their endpoints, the documented variables and flags that
//! configure it, and which variables of the user's shell would otherwise decide
//! its provider, model or credentials. Agents are looked up by id in one table
//! ([`adapter`]); nothing else in the app tells agents apart.
//!
//! [`configure`] applies an adapter to a launch. The precedence rule (ADR 0015):
//! when the app configures a session's provider, every variable the adapter
//! controls is removed from the inherited environment and the adapter's are set,
//! so the agent sees the app's provider configuration and nothing else; the rest
//! of the environment is unchanged. When the app configures nothing, the
//! environment is inherited as it is. The two are never mixed.
//!
//! An adapter can also give its agent MCP servers for one session
//! (docs/mcp.md, ADR 0017): [`attach_mcp`] adds, through the agent's documented
//! per-session mechanism, where each server is. The app starts stdio servers
//! itself; the agent is told to run the app's bridge to reach them, so it never
//! sees their command, their variables or their secrets. An agent whose adapter
//! does not implement it, or that has no adapter, gets no MCP servers.

mod claude_code;
mod opencode;

use std::fmt;
use std::path::PathBuf;

use x8ai_core::agent::SessionConfiguration;
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::McpTransportKind;
use x8ai_core::model::{
    CredentialState, MODEL_ID_RULE, ModelProviderDefinition, ModelSelection, ProviderAuth,
    ProviderEndpoint, is_model_id,
};
use x8ai_secrets::SecretValue;

pub use claude_code::ClaudeCode;
pub use opencode::OpenCode;

use crate::runtime::{LaunchPlan, ProviderRoute};

/// What an adapter sets up for one session.
#[derive(Clone, PartialEq, Eq)]
pub struct Configuration {
    /// Variables to set. May hold the credential.
    pub env: Vec<(String, String)>,
    /// Arguments added after the definition's own, such as `--model <id>`.
    pub args: Vec<String>,
    /// Where the agent will send its model requests, and the credential.
    pub endpoint: String,
}

impl fmt::Debug for Configuration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Configuration")
            .field("env", &names(&self.env))
            .field("args", &self.args)
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

/// The names of `env`, for anything printed: values may be credentials.
pub(crate) fn names(env: &[(String, String)]) -> Vec<&str> {
    env.iter().map(|(name, _)| name.as_str()).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigureError {
    #[error("{agent} cannot use {provider}: {reason}")]
    Unsupported {
        agent: String,
        provider: String,
        reason: String,
    },
    #[error("the app cannot configure {0}'s model; run it with its own configuration")]
    NoAdapter(String),
    #[error("no API key for {0} is saved; add one in Models")]
    MissingCredential(String),
    #[error("model id {0:?} {MODEL_ID_RULE}")]
    InvalidModel(String),
    #[error("{agent} cannot use MCP servers from the app: {reason}")]
    McpUnsupported { agent: String, reason: String },
}

/// An MCP server as an agent is told about it for one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentMcpServer {
    pub id: IntegrationId,
    pub transport: AgentMcpTransport,
}

impl AgentMcpServer {
    /// The name the agent shows it under: prefixed, so it cannot collide with
    /// servers in the agent's own configuration.
    pub fn name(&self) -> String {
        format!("x8ai-{}", self.id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentMcpTransport {
    /// The app's bridge to the server's socket: a program and its arguments.
    /// Never the server's own command.
    Stdio {
        command: PathBuf,
        args: Vec<String>,
    },
    StreamableHttp {
        url: String,
    },
}

impl AgentMcpTransport {
    pub fn kind(&self) -> McpTransportKind {
        match self {
            Self::Stdio { .. } => McpTransportKind::Stdio,
            Self::StreamableHttp { .. } => McpTransportKind::StreamableHttp,
        }
    }
}

/// What an adapter adds so its agent uses a session's MCP servers.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct McpConfiguration {
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
}

impl fmt::Debug for McpConfiguration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpConfiguration")
            .field("env", &names(&self.env))
            .field("args", &self.args)
            .finish()
    }
}

/// One agent's knowledge of how it is configured. Every implementation cites the
/// agent's documentation for what it sets; nothing is guessed.
pub trait AgentAdapter: Send + Sync {
    /// The agent definition's id.
    fn agent(&self) -> &'static str;
    /// The endpoint of `provider` the agent would use, or why it cannot.
    fn endpoint<'p>(
        &self,
        provider: &'p ModelProviderDefinition,
    ) -> Result<&'p ProviderEndpoint, String>;
    /// Whether `variable` decides the agent's provider, endpoint, credentials or
    /// model. These are what an app-configured session replaces.
    fn controls(&self, variable: &str) -> bool;
    /// The variables and arguments that make the agent use `model` from
    /// `provider` at `endpoint`, authenticating with `credential` if the provider
    /// needs one. `model` is a checked model id.
    fn configure(
        &self,
        provider: &ModelProviderDefinition,
        endpoint: &ProviderEndpoint,
        model: &str,
        credential: Option<&SecretValue>,
    ) -> Configuration;
    /// Whether the app can give the agent MCP servers for one session, and why
    /// not. Unsupported unless the adapter implements it.
    fn mcp(&self) -> Result<(), String> {
        Err(
            "the app does not know a documented way to give it MCP servers for one session"
                .to_owned(),
        )
    }
    /// The variables and arguments that give the agent `servers` for this session
    /// only. `env` is the launch's environment so far. Called only when
    /// [`mcp`](Self::mcp) says it is supported.
    fn configure_mcp(
        &self,
        servers: &[AgentMcpServer],
        env: &[(String, String)],
    ) -> McpConfiguration {
        let _ = (servers, env);
        McpConfiguration::default()
    }
}

static ADAPTERS: [&dyn AgentAdapter; 2] = [&ClaudeCode, &OpenCode];

/// The adapter for the agent with this id. Codex and other agents have none yet:
/// they run with their own configuration only.
pub fn adapter(agent: &str) -> Option<&'static dyn AgentAdapter> {
    ADAPTERS.iter().copied().find(|a| a.agent() == agent)
}

/// Whether the agent can use `provider`, and why not.
pub fn support(agent: &str, provider: &ModelProviderDefinition) -> Result<(), String> {
    let adapter = adapter(agent).ok_or_else(|| {
        "the app cannot configure this agent's model yet; it uses its own configuration".to_owned()
    })?;
    adapter.endpoint(provider).map(|_| ())
}

/// The names of variables in `env` that the agent would read for its provider
/// configuration, sorted. For sessions that use the agent's own configuration.
pub fn shell_variables(agent: &str, env: &[(String, String)]) -> Vec<String> {
    let Some(adapter) = adapter(agent) else {
        return Vec::new();
    };
    let mut found: Vec<String> = env
        .iter()
        .filter(|(name, _)| adapter.controls(name))
        .map(|(name, _)| name.clone())
        .collect();
    found.sort();
    found.dedup();
    found
}

/// Whether the agent (by id, declaring `transports` in its definition) can be
/// given MCP servers by the app, and why not.
pub fn mcp_support(agent: &str, transports: &[McpTransportKind]) -> Result<(), String> {
    let adapter = adapter(agent).ok_or_else(|| {
        "the app has no adapter for this agent, so it gets no MCP servers from the app".to_owned()
    })?;
    if transports.is_empty() {
        return Err("its definition declares no MCP transports".to_owned());
    }
    adapter.mcp()
}

/// Gives the agent of `plan` these MCP servers, for this launch only, through its
/// adapter. The agent's own and project MCP configuration are left as they are.
/// Refused if the agent cannot use them; nothing is added then. `transports`
/// are the MCP transports the agent's definition declares.
pub fn attach_mcp(
    mut plan: LaunchPlan,
    transports: &[McpTransportKind],
    servers: &[AgentMcpServer],
) -> Result<LaunchPlan, ConfigureError> {
    if servers.is_empty() {
        return Ok(plan);
    }
    let unsupported = |reason: String| ConfigureError::McpUnsupported {
        agent: plan.name.clone(),
        reason,
    };
    mcp_support(plan.agent.as_str(), transports).map_err(unsupported)?;
    if let Some(server) = servers
        .iter()
        .find(|s| !transports.contains(&s.transport.kind()))
    {
        return Err(unsupported(format!(
            "{} needs a transport it does not support",
            server.id
        )));
    }
    let adapter = adapter(plan.agent.as_str()).expect("supported");
    let configuration = adapter.configure_mcp(servers, &plan.env);
    for (name, value) in configuration.env {
        plan.env.retain(|(n, _)| *n != name);
        plan.env.push((name, value));
    }
    plan.extra_args.extend(configuration.args);
    plan.mcp = servers.iter().map(|s| s.id.clone()).collect();
    Ok(plan)
}

/// Points `plan` at `model` from `provider`. `credential` is the provider's saved
/// key; it is required when the provider authenticates with one. The inherited
/// environment loses every variable the agent's adapter controls, and gains the
/// adapter's (the precedence rule above). The provider and endpoint become part of
/// what the user approves; the model does not.
pub fn configure(
    mut plan: LaunchPlan,
    provider: &ModelProviderDefinition,
    model: &str,
    credential: Option<&SecretValue>,
) -> Result<LaunchPlan, ConfigureError> {
    let (adapter, endpoint) = prepare(plan.agent.as_str(), &plan.name, provider, model)?;
    let (credential, state) = match &provider.auth {
        ProviderAuth::None => (None, CredentialState::NotNeeded),
        ProviderAuth::ApiKey { .. } => match credential {
            Some(credential) => (Some(credential), CredentialState::InKeychain),
            None => return Err(ConfigureError::MissingCredential(provider.name.clone())),
        },
    };
    let configuration = adapter.configure(provider, endpoint, model, credential);

    let overridden = shell_variables(adapter.agent(), &plan.env);
    plan.env.retain(|(name, _)| !adapter.controls(name));
    for (name, value) in configuration.env {
        plan.env.retain(|(n, _)| *n != name);
        plan.env.push((name, value));
    }
    plan.extra_args = configuration.args;
    plan.provider = Some(ProviderRoute {
        provider: provider.id.clone(),
        endpoint: configuration.endpoint.clone(),
    });
    plan.model = Some(ModelSelection {
        provider: provider.id.clone(),
        model: model.to_owned(),
    });
    plan.configuration =
        app_configuration(provider, model, configuration.endpoint, state, overridden);
    Ok(plan)
}

/// What a session of `agent` using `model` from `provider` would be configured
/// with, without touching its credential: for showing a session that has not run
/// yet in this run of the app. `env` is the inherited environment.
pub fn describe(
    agent: &str,
    provider: &ModelProviderDefinition,
    model: &str,
    env: &[(String, String)],
    credential: CredentialState,
) -> Result<SessionConfiguration, ConfigureError> {
    let (adapter, endpoint) = prepare(agent, agent, provider, model)?;
    let endpoint = adapter.configure(provider, endpoint, model, None).endpoint;
    Ok(app_configuration(
        provider,
        model,
        endpoint,
        credential,
        shell_variables(agent, env),
    ))
}

fn prepare<'p>(
    agent: &str,
    agent_name: &str,
    provider: &'p ModelProviderDefinition,
    model: &str,
) -> Result<(&'static dyn AgentAdapter, &'p ProviderEndpoint), ConfigureError> {
    let adapter = adapter(agent).ok_or_else(|| ConfigureError::NoAdapter(agent_name.to_owned()))?;
    if !is_model_id(model) {
        return Err(ConfigureError::InvalidModel(model.to_owned()));
    }
    let endpoint = adapter
        .endpoint(provider)
        .map_err(|reason| ConfigureError::Unsupported {
            agent: agent_name.to_owned(),
            provider: provider.name.clone(),
            reason,
        })?;
    Ok((adapter, endpoint))
}

fn app_configuration(
    provider: &ModelProviderDefinition,
    model: &str,
    endpoint: String,
    credential: CredentialState,
    overridden_shell_variables: Vec<String>,
) -> SessionConfiguration {
    SessionConfiguration::App {
        provider: provider.id.clone(),
        provider_name: provider.name.clone(),
        model: model.to_owned(),
        endpoint,
        credential,
        overridden_shell_variables,
    }
}

/// The endpoint of `provider` that speaks one of `apis`, in the order given.
fn endpoint_for<'p>(
    provider: &'p ModelProviderDefinition,
    apis: &[x8ai_core::model::ProviderApi],
) -> Option<&'p ProviderEndpoint> {
    apis.iter()
        .find_map(|api| provider.endpoints.iter().find(|e| e.api == *api))
}
