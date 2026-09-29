//! Which MCP servers the user allowed to run in which workspace, in
//! `mcp-approvals.json` (0600), outside every workspace.
//!
//! An approval pins exactly what would run: for stdio, the resolved executable
//! and every argument; for HTTP, the URL; for both, the variables and where their
//! values come from. Any change to these is a different server that needs a new
//! approval; its name, description, scope and enabled state are not (ADR 0017).

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use x8ai_core::mcp::McpEnvVar;

use crate::files::{read_json, write_private};

/// What an approval covers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Material {
    pub transport: MaterialTransport,
    pub env: Vec<McpEnvVar>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MaterialTransport {
    /// The executable as found (absolute), and its arguments.
    Stdio {
        program: PathBuf,
        args: Vec<String>,
    },
    StreamableHttp {
        url: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Approved {
    server: String,
    material: Material,
    /// Milliseconds since the Unix epoch.
    at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    root: PathBuf,
    servers: Vec<Approved>,
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    workspaces: Vec<Entry>,
}

const VERSION: u32 = 1;

#[derive(Debug)]
pub struct Approvals {
    file: PathBuf,
    entries: Vec<Entry>,
}

impl Approvals {
    pub fn load(file: PathBuf) -> (Self, Option<String>) {
        let (parsed, warning) = read_json::<File>(&file);
        let mut entries = match parsed {
            Some(parsed) if parsed.version == VERSION => parsed.workspaces,
            _ => Vec::new(),
        };
        // Only absolute roots and absolute executables mean anything.
        entries.retain(|e| e.root.is_absolute());
        for entry in &mut entries {
            entry.servers.retain(|a| match &a.material.transport {
                MaterialTransport::Stdio { program, .. } => program.is_absolute(),
                MaterialTransport::StreamableHttp { .. } => true,
            });
        }
        (Self { file, entries }, warning)
    }

    /// Whether exactly this configuration of `server` is approved in `root`.
    pub fn is_approved(&self, root: &Path, server: &str, material: &Material) -> bool {
        self.entries.iter().any(|e| {
            e.root == root
                && e.servers
                    .iter()
                    .any(|a| a.server == server && a.material == *material)
        })
    }

    /// Whether some configuration of `server` was approved in `root`, so a
    /// refusal can say that it changed.
    pub fn was_approved(&self, root: &Path, server: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.root == root && e.servers.iter().any(|a| a.server == server))
    }

    /// Records approvals in `root`, each replacing an earlier one for the same
    /// server there.
    pub fn approve(&mut self, root: &Path, servers: &[(&str, &Material)]) -> std::io::Result<()> {
        let at = now();
        let index = match self.entries.iter().position(|e| e.root == root) {
            Some(index) => index,
            None => {
                self.entries.push(Entry {
                    root: root.to_owned(),
                    servers: Vec::new(),
                });
                self.entries.len() - 1
            }
        };
        let entry = &mut self.entries[index];
        for (server, material) in servers {
            entry.servers.retain(|a| a.server != *server);
            entry.servers.push(Approved {
                server: (*server).to_owned(),
                material: (*material).clone(),
                at,
            });
        }
        self.save()
    }

    /// Forgets every MCP approval in `root`, as when its trust is removed.
    pub fn revoke_all(&mut self, root: &Path) -> std::io::Result<()> {
        let before = self.entries.len();
        self.entries.retain(|e| e.root != root);
        if self.entries.len() == before {
            return Ok(());
        }
        self.save()
    }

    /// Forgets `server` everywhere, as when it is removed from the registry.
    pub fn forget(&mut self, server: &str) -> std::io::Result<()> {
        let before: usize = self.entries.iter().map(|e| e.servers.len()).sum();
        for entry in &mut self.entries {
            entry.servers.retain(|a| a.server != server);
        }
        self.entries.retain(|e| !e.servers.is_empty());
        let after: usize = self.entries.iter().map(|e| e.servers.len()).sum();
        if before == after {
            return Ok(());
        }
        self.save()
    }

    fn save(&self) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(&File {
            version: VERSION,
            workspaces: self.entries.clone(),
        })
        .expect("serializable");
        write_private(&self.file, &json)
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
