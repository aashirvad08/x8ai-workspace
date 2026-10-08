//! A session's MCP servers as a host chooses them, the same in the app and in
//! `x8ai` (docs/mcp.md): which servers a new session gets, which ones a later
//! run still uses, and which can run now. The agent's side (whether it can use
//! MCP servers, over which transports) comes from its adapter, through
//! [`AgentMcp`], so this crate does not depend on the agents.

use std::path::Path;

use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{McpServer, McpTransportKind};
use x8ai_secrets::SecretStore;

use crate::environment::secret_account;
use crate::session::{Prepared, SelectError, attach, prepare, still_attached};

/// What an agent can do with MCP servers.
pub struct AgentMcp<'a> {
    pub name: &'a str,
    /// Its adapter's answer: whether it can be given MCP servers at all.
    pub support: Result<(), String>,
    /// The transports its definition lists.
    pub transports: &'a [McpTransportKind],
}

/// A launch's servers: every one attached to the session, those that will
/// run, exactly as they would, and the others with the reason.
#[derive(Debug, Default)]
pub struct Selection {
    pub attached: Vec<IntegrationId>,
    pub prepared: Vec<Prepared>,
    pub skipped: Vec<(IntegrationId, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ChoiceError {
    /// The agent cannot be given MCP servers, and some were chosen for it.
    #[error("{agent} cannot use MCP servers: {reason}")]
    Unsupported { agent: String, reason: String },
    #[error("{0}")]
    Select(#[from] SelectError),
    #[error("{agent} cannot use {server}: it does not support that transport")]
    Transport { agent: String, server: String },
}

/// The servers a new session in `root` gets: every enabled global and
/// workspace server the agent can use, and the session servers `chosen`.
/// Choosing servers for an agent that cannot use them is refused; the others
/// are simply not attached to it.
pub fn for_new_session(
    servers: &[McpServer],
    agent: &AgentMcp<'_>,
    root: &Path,
    chosen: &[IntegrationId],
    secrets: &dyn SecretStore,
    path: Option<&str>,
) -> Result<Selection, ChoiceError> {
    if let Err(reason) = &agent.support {
        if chosen.is_empty() {
            return Ok(Selection::default());
        }
        return Err(ChoiceError::Unsupported {
            agent: agent.name.to_owned(),
            reason: reason.clone(),
        });
    }
    let mut attached = Vec::new();
    for server in attach(servers, root, chosen)? {
        if agent.transports.contains(&server.transport.kind()) {
            attached.push(server.clone());
        } else if chosen.contains(&server.id) {
            return Err(ChoiceError::Transport {
                agent: agent.name.to_owned(),
                server: server.name.clone(),
            });
        }
    }
    let (prepared, skipped) = prepare_all(&attached, secrets, path);
    Ok(Selection {
        attached: attached.iter().map(|s| s.id.clone()).collect(),
        prepared,
        skipped,
    })
}

/// The servers another run of a session uses: those it was created with that
/// still exist, are enabled, and can run. Never one it did not have.
pub fn for_run(
    servers: &[McpServer],
    agent: &AgentMcp<'_>,
    root: &Path,
    attached: &[IntegrationId],
    secrets: &dyn SecretStore,
    path: Option<&str>,
) -> Selection {
    let (kept, mut skipped) = still_attached(servers, root, attached);
    let mut usable = Vec::new();
    for server in kept {
        match &agent.support {
            Err(reason) => skipped.push((server.id.clone(), reason.clone())),
            Ok(()) if !agent.transports.contains(&server.transport.kind()) => {
                skipped.push((
                    server.id.clone(),
                    "the agent does not support its transport".to_owned(),
                ));
            }
            Ok(()) => usable.push(server.clone()),
        }
    }
    let (prepared, more) = prepare_all(&usable, secrets, path);
    skipped.extend(more);
    Selection {
        attached: attached.to_vec(),
        prepared,
        skipped,
    }
}

/// Each server as it would run; one whose command is not found or whose secret
/// is not saved is left out, with the reason. Nothing half configured runs.
pub fn prepare_all(
    servers: &[McpServer],
    secrets: &dyn SecretStore,
    path: Option<&str>,
) -> (Vec<Prepared>, Vec<(IntegrationId, String)>) {
    let mut prepared = Vec::new();
    let mut skipped = Vec::new();
    for server in servers {
        let missing: Vec<&str> = server
            .secret_names()
            .filter(|name| {
                !secrets
                    .contains(&secret_account(server.id.as_str(), name))
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
        match prepare(server, path) {
            Ok(p) => prepared.push(p),
            Err(error) => skipped.push((server.id.clone(), error.to_string())),
        }
    }
    (prepared, skipped)
}
