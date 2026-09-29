//! Which servers a session gets, what exactly each would run, and the one gate
//! before anything runs: trust and approval.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{McpScope, McpServer, McpServerTransport};
use x8ai_workspace::TrustStore;

use crate::approvals::{Approvals, Material, MaterialTransport};

/// The servers a new session in `root` gets: every enabled global server, every
/// enabled server of this workspace, and the enabled session servers in
/// `chosen`. In registry order.
pub fn attach<'r>(
    servers: &'r [McpServer],
    root: &Path,
    chosen: &[IntegrationId],
) -> Result<Vec<&'r McpServer>, SelectError> {
    for id in chosen {
        let server = servers
            .iter()
            .find(|s| s.id == *id)
            .ok_or_else(|| SelectError::Unknown(id.to_string()))?;
        if server.scope != McpScope::Session {
            return Err(SelectError::NotSessionScoped(server.name.clone()));
        }
        if !server.enabled {
            return Err(SelectError::Disabled(server.name.clone()));
        }
    }
    Ok(servers
        .iter()
        .filter(|s| s.enabled)
        .filter(|s| match &s.scope {
            McpScope::Global => true,
            McpScope::Workspace { root: r } => Path::new(r) == root,
            McpScope::Session => chosen.contains(&s.id),
        })
        .collect())
}

/// For another run of a session: its servers that still exist, are still
/// enabled and still belong to its workspace. The others are left out, each
/// with the reason. Servers the session did not have are never added.
pub fn still_attached<'r>(
    servers: &'r [McpServer],
    root: &Path,
    attached: &[IntegrationId],
) -> (Vec<&'r McpServer>, Vec<(IntegrationId, String)>) {
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    for id in attached {
        match servers.iter().find(|s| s.id == *id) {
            None => skipped.push((id.clone(), "removed from the app".to_owned())),
            Some(s) if !s.enabled => skipped.push((id.clone(), "disabled".to_owned())),
            Some(McpServer {
                scope: McpScope::Workspace { root: r },
                ..
            }) if Path::new(r) != root => {
                skipped.push((id.clone(), "now belongs to another folder".to_owned()))
            }
            Some(s) => kept.push(s),
        }
    }
    (kept, skipped)
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SelectError {
    #[error("no MCP server {0:?}")]
    Unknown(String),
    #[error("{0} is attached to every session it applies to; it cannot be chosen per session")]
    NotSessionScoped(String),
    #[error("{0} is disabled")]
    Disabled(String),
}

/// A server with exactly what would run: what the user approves, and nothing else
/// is started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub server: McpServer,
    pub material: Material,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrepareError {
    #[error("{name}: {problem}")]
    Invalid { name: String, problem: String },
    #[error("{name}: `{command}` was not found on your PATH; install it yourself")]
    NotFound { name: String, command: String },
}

/// Resolves the server's command on the login `PATH` (`path`), as agents are.
pub fn prepare(server: &McpServer, path: Option<&str>) -> Result<Prepared, PrepareError> {
    server.validate().map_err(|e| PrepareError::Invalid {
        name: server.name.clone(),
        problem: e.to_string(),
    })?;
    let transport = match &server.transport {
        McpServerTransport::Stdio { command, args } => MaterialTransport::Stdio {
            program: find_program(command, path).ok_or_else(|| PrepareError::NotFound {
                name: server.name.clone(),
                command: command.clone(),
            })?,
            args: args.clone(),
        },
        McpServerTransport::StreamableHttp { url } => {
            MaterialTransport::StreamableHttp { url: url.clone() }
        }
    };
    Ok(Prepared {
        server: server.clone(),
        material: Material {
            transport,
            env: server.env.clone(),
        },
    })
}

/// An absolute path as it is, or a bare name in the absolute directories of
/// `path`: the first regular executable file. The same rules as for agents.
pub fn find_program(program: &str, path: Option<&str>) -> Option<PathBuf> {
    let executable = |p: &Path| {
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        let candidate = Path::new(program);
        return (candidate.is_absolute() && executable(candidate)).then(|| candidate.to_owned());
    }
    path?
        .split(':')
        .map(Path::new)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|candidate| executable(candidate))
}

/// Why servers cannot run.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Denied {
    #[error("MCP servers run only in folders you trust; {} is not trusted", .0.display())]
    Untrusted(PathBuf),
    #[error("not allowed in this folder yet{}: {}", if *.changed { " (changed since it was allowed)" } else { "" }, .names.join(", "))]
    NotApproved { names: Vec<String>, changed: bool },
}

/// Servers that passed [`authorize`]. Only that function makes one, and only
/// with it can servers be started.
#[derive(Debug)]
pub struct Authorized<'a> {
    pub(crate) root: &'a Path,
    pub(crate) servers: &'a [Prepared],
}

impl Authorized<'_> {
    /// The workspace they were authorized for.
    pub fn root(&self) -> &Path {
        self.root
    }
}

/// Allows the servers only if `root` is trusted and each is approved there,
/// exactly as prepared.
pub fn authorize<'a>(
    root: &'a Path,
    servers: &'a [Prepared],
    trust: &TrustStore,
    approvals: &Approvals,
) -> Result<Authorized<'a>, Denied> {
    if !trust.is_trusted(root) {
        return Err(Denied::Untrusted(root.to_owned()));
    }
    let missing = unapproved(root, servers, approvals);
    if !missing.is_empty() {
        return Err(Denied::NotApproved {
            changed: missing
                .iter()
                .any(|p| approvals.was_approved(root, p.server.id.as_str())),
            names: missing.iter().map(|p| p.server.name.clone()).collect(),
        });
    }
    Ok(Authorized { root, servers })
}

/// The servers of `servers` not approved in `root` as they are now.
pub fn unapproved<'a>(
    root: &Path,
    servers: &'a [Prepared],
    approvals: &Approvals,
) -> Vec<&'a Prepared> {
    servers
        .iter()
        .filter(|p| !approvals.is_approved(root, p.server.id.as_str(), &p.material))
        .collect()
}
