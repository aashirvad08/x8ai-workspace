//! The MCP servers the user configured, in `mcp-servers.json` (0600).
//!
//! Only what `McpServer` holds: names, a command and its arguments or a URL,
//! variable names and where their values come from, whether it is enabled, and
//! its scope. Never a secret value; those are in the Keychain. A damaged file is
//! set aside and the registry starts empty; an entry that no longer validates is
//! dropped with a warning.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use x8ai_core::definition::DefinitionError;
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{McpScope, McpScopeKind, McpServer, McpServerInput};

use crate::files::{read_json, write_private};

/// Most servers kept.
pub const MAX_SERVERS: usize = 100;
const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    servers: Vec<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(#[from] DefinitionError),
    #[error("no MCP server {0:?}")]
    NotFound(String),
    #[error("open a folder first: a workspace server belongs to the open folder")]
    NoWorkspace,
    #[error("at most {MAX_SERVERS} MCP servers can be added")]
    Full,
    #[error("{path}: {detail}")]
    Io { path: String, detail: String },
}

#[derive(Debug)]
pub struct Registry {
    file: PathBuf,
    servers: Vec<McpServer>,
}

impl Registry {
    /// Loads the registry; a missing file is empty. Returns warnings for a
    /// damaged file or dropped entries.
    pub fn load(file: PathBuf) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let (parsed, warning) = read_json::<File>(&file);
        warnings.extend(warning);
        let mut servers: Vec<McpServer> = Vec::new();
        match parsed {
            Some(parsed) if parsed.version == VERSION => {
                for value in parsed.servers {
                    let server = serde_json::from_value::<McpServer>(value)
                        .map_err(|e| e.to_string())
                        .and_then(|s| s.validate().map(|()| s).map_err(|e| e.to_string()));
                    match server {
                        Ok(server) if servers.iter().all(|s| s.id != server.id) => {
                            servers.push(server)
                        }
                        Ok(server) => warnings.push(format!(
                            "MCP server {} is listed twice; the second was ignored",
                            server.id
                        )),
                        Err(reason) => warnings.push(format!(
                            "an MCP server in {} was ignored: {reason}",
                            file.display()
                        )),
                    }
                }
            }
            Some(parsed) => warnings.push(format!(
                "{} has version {}, which this app does not read; starting empty",
                file.display(),
                parsed.version
            )),
            None => {}
        }
        servers.truncate(MAX_SERVERS);
        (Self { file, servers }, warnings)
    }

    pub fn servers(&self) -> &[McpServer] {
        &self.servers
    }

    pub fn get(&self, id: &str) -> Option<&McpServer> {
        self.servers.iter().find(|s| s.id.as_str() == id)
    }

    /// Adds a server made from `input`, with an id made from its name.
    /// `workspace` is the open folder, for a workspace scope.
    pub fn add(
        &mut self,
        input: &McpServerInput,
        workspace: Option<&Path>,
    ) -> Result<McpServer, Error> {
        input.validate()?;
        if self.servers.len() >= MAX_SERVERS {
            return Err(Error::Full);
        }
        let server = McpServer {
            id: self.new_id(&input.name),
            name: input.name.trim().to_owned(),
            description: input.description.trim().to_owned(),
            transport: input.transport.clone(),
            env: input.env.clone(),
            enabled: input.enabled,
            scope: scope(input.scope, workspace, None)?,
        };
        server.validate()?;
        self.servers.push(server.clone());
        self.save()?;
        Ok(server)
    }

    /// Replaces the server's configuration. Its id stays. Returns the old and
    /// the new entry.
    pub fn update(
        &mut self,
        id: &str,
        input: &McpServerInput,
        workspace: Option<&Path>,
    ) -> Result<(McpServer, McpServer), Error> {
        input.validate()?;
        let index = self.index(id)?;
        let old = self.servers[index].clone();
        let server = McpServer {
            id: old.id.clone(),
            name: input.name.trim().to_owned(),
            description: input.description.trim().to_owned(),
            transport: input.transport.clone(),
            env: input.env.clone(),
            enabled: input.enabled,
            scope: scope(input.scope, workspace, Some(&old.scope))?,
        };
        server.validate()?;
        self.servers[index] = server.clone();
        self.save()?;
        Ok((old, server))
    }

    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<McpServer, Error> {
        let index = self.index(id)?;
        self.servers[index].enabled = enabled;
        self.save()?;
        Ok(self.servers[index].clone())
    }

    pub fn remove(&mut self, id: &str) -> Result<McpServer, Error> {
        let index = self.index(id)?;
        let removed = self.servers.remove(index);
        self.save()?;
        Ok(removed)
    }

    fn index(&self, id: &str) -> Result<usize, Error> {
        self.servers
            .iter()
            .position(|s| s.id.as_str() == id)
            .ok_or_else(|| Error::NotFound(id.to_owned()))
    }

    /// `github`, `github-2`, …: lowercase letters, digits and dashes from the name.
    fn new_id(&self, name: &str) -> IntegrationId {
        let mut slug = String::new();
        for c in name.trim().chars().flat_map(char::to_lowercase) {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                slug.push(c);
            } else if !slug.ends_with('-') && !slug.is_empty() {
                slug.push('-');
            }
        }
        let slug: String = slug
            .trim_start_matches(|c: char| !c.is_ascii_lowercase())
            .chars()
            .take(48)
            .collect();
        let slug = slug.trim_end_matches('-');
        let base = if slug.is_empty() { "server" } else { slug };
        (1..)
            .map(|n| {
                if n == 1 {
                    base.to_owned()
                } else {
                    format!("{base}-{n}")
                }
            })
            .find(|candidate| self.servers.iter().all(|s| s.id.as_str() != candidate))
            .and_then(|id| IntegrationId::new(id).ok())
            .expect("a free id")
    }

    fn save(&self) -> Result<(), Error> {
        let file = File {
            version: VERSION,
            servers: self
                .servers
                .iter()
                .map(|s| serde_json::to_value(s).expect("serializable"))
                .collect(),
        };
        let json = serde_json::to_vec_pretty(&file).expect("serializable");
        write_private(&self.file, &json).map_err(|e| Error::Io {
            path: self.file.display().to_string(),
            detail: e.to_string(),
        })
    }
}

/// The scope for `kind`: a workspace scope is the open folder, or the folder it
/// already had when no folder is open.
fn scope(
    kind: McpScopeKind,
    workspace: Option<&Path>,
    previous: Option<&McpScope>,
) -> Result<McpScope, Error> {
    Ok(match kind {
        McpScopeKind::Global => McpScope::Global,
        McpScopeKind::Session => McpScope::Session,
        McpScopeKind::Workspace => match (workspace, previous) {
            (Some(root), _) => McpScope::Workspace {
                root: root.display().to_string(),
            },
            (None, Some(previous @ McpScope::Workspace { .. })) => previous.clone(),
            (None, _) => return Err(Error::NoWorkspace),
        },
    })
}
