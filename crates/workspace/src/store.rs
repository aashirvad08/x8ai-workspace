//! What the app remembers about workspaces between launches: which folders were
//! opened recently, which the user trusts, and which agents the user allowed to
//! run in which folder. Nothing else. No file names, no contents, no settings from
//! inside the folder, and nothing the folder itself can change.
//!
//! Each store is a small JSON file in the app's data directory, readable only by
//! the user (mode 0600), and replaced atomically on every change, so a crash
//! never leaves it half-written. Only the native side writes these files; the
//! webview cannot.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use x8ai_core::workspace::RecentWorkspace;

use crate::Error;

/// Most workspaces remembered in the recent list.
pub const MAX_RECENT: usize = 15;

/// A store file larger than this is not ours, and is treated as corrupt.
const MAX_STORE_BYTES: u64 = 1024 * 1024;

/// Recently opened workspaces, most recent first.
#[derive(Debug)]
pub struct RecentWorkspaces {
    file: PathBuf,
    entries: Vec<Entry>,
}

/// Folders the user has explicitly trusted.
#[derive(Debug)]
pub struct TrustStore {
    file: PathBuf,
    entries: Vec<Entry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    root: PathBuf,
    /// Milliseconds since the Unix epoch: when opened, or when trusted.
    at: u64,
}

/// Agents the user allowed to run in a folder (ADR 0012).
#[derive(Debug)]
pub struct ApprovalStore {
    file: PathBuf,
    entries: Vec<ApprovalEntry>,
}

/// What an approval covers: one agent, launched as exactly this program with
/// these arguments, in one folder, using its own model configuration or exactly
/// this provider at this endpoint. A different program found on `PATH` later,
/// changed arguments, another provider or another endpoint is not covered and
/// must be approved again (ADR 0015, "Material changes").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Approval<'a> {
    pub root: &'a Path,
    pub agent: &'a str,
    pub program: &'a Path,
    pub args: &'a [String],
    /// `None`: the agent's own configuration.
    pub provider: Option<ApprovedProvider<'a>>,
}

/// A provider the app configures for an agent, and where the agent's requests,
/// and its credential, go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApprovedProvider<'a> {
    pub id: &'a str,
    pub endpoint: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovalEntry {
    root: PathBuf,
    agents: Vec<ApprovedAgent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ApprovedAgent {
    id: String,
    program: PathBuf,
    args: Vec<String>,
    /// Absent in approvals of the agent's own configuration, and in every
    /// approval made before providers existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    provider: Option<ProviderEntry>,
    /// Milliseconds since the Unix epoch.
    at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProviderEntry {
    id: String,
    endpoint: String,
}

impl ApprovedAgent {
    /// Whether this approval and `approval` are for the same agent and provider:
    /// approving one replaces the other.
    fn same_route(&self, approval: &Approval<'_>) -> bool {
        self.id == approval.agent
            && self.provider.as_ref().map(|p| p.id.as_str()) == approval.provider.map(|p| p.id)
    }

    fn covers(&self, approval: &Approval<'_>) -> bool {
        self.same_route(approval)
            && self.program == approval.program
            && self.args == approval.args
            && self.provider.as_ref().map(|p| p.endpoint.as_str())
                == approval.provider.map(|p| p.endpoint)
    }
}

/// Every store file: a version and one entry per workspace.
#[derive(Serialize, Deserialize)]
struct StoreFile<T> {
    version: u32,
    workspaces: Vec<T>,
}

/// An entry that belongs to one workspace, identified by its absolute root, or
/// (a space's) to the workspace with no folder.
pub(crate) trait Rooted {
    fn root(&self) -> Option<&Path>;
}

impl Rooted for Entry {
    fn root(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

impl Rooted for ApprovalEntry {
    fn root(&self) -> Option<&Path> {
        Some(&self.root)
    }
}

const STORE_VERSION: u32 = 1;

impl RecentWorkspaces {
    /// Loads the list. A missing file is an empty list; see [`load`] for the
    /// handling of a damaged one.
    pub fn load(file: PathBuf) -> (Self, Option<String>) {
        let (entries, warning) = load(&file);
        (Self { file, entries }, warning)
    }

    /// Moves `root` to the front of the list, or adds it there.
    pub fn record(&mut self, root: &Path) -> Result<(), Error> {
        self.entries.retain(|e| e.root != root);
        self.entries.insert(
            0,
            Entry {
                root: root.to_owned(),
                at: now(),
            },
        );
        self.entries.truncate(MAX_RECENT);
        save(&self.file, &self.entries)
    }

    pub fn remove(&mut self, root: &Path) -> Result<(), Error> {
        let before = self.entries.len();
        self.entries.retain(|e| e.root != root);
        if self.entries.len() == before {
            return Ok(());
        }
        save(&self.file, &self.entries)
    }

    pub fn contains(&self, root: &Path) -> bool {
        self.entries.iter().any(|e| e.root == root)
    }

    /// The list, noting which folders still exist.
    pub fn list(&self) -> Vec<RecentWorkspace> {
        self.entries
            .iter()
            .filter_map(|e| {
                let root = e.root.to_str()?.to_owned();
                let name = e
                    .root
                    .file_name()
                    .map_or_else(|| root.clone(), |n| n.to_string_lossy().into_owned());
                Some(RecentWorkspace {
                    id: None,
                    available: e.root.is_dir(),
                    root,
                    name,
                })
            })
            .collect()
    }
}

impl TrustStore {
    pub fn load(file: PathBuf) -> (Self, Option<String>) {
        let (entries, warning) = load(&file);
        (Self { file, entries }, warning)
    }

    /// Trust applies to exactly this folder. It is not inherited by its parent or
    /// by other folders inside it that are opened on their own.
    pub fn is_trusted(&self, root: &Path) -> bool {
        self.entries.iter().any(|e| e.root == root)
    }

    pub fn set(&mut self, root: &Path, trusted: bool) -> Result<(), Error> {
        if self.is_trusted(root) == trusted {
            return Ok(());
        }
        if trusted {
            self.entries.push(Entry {
                root: root.to_owned(),
                at: now(),
            });
        } else {
            self.entries.retain(|e| e.root != root);
        }
        save(&self.file, &self.entries)
    }
}

impl ApprovalStore {
    pub fn load(file: PathBuf) -> (Self, Option<String>) {
        let (mut entries, warning) = load::<ApprovalEntry>(&file);
        for entry in &mut entries {
            entry.agents.retain(|a| a.program.is_absolute());
        }
        (Self { file, entries }, warning)
    }

    /// Whether exactly this launch was approved in exactly this folder.
    pub fn is_approved(&self, approval: &Approval<'_>) -> bool {
        self.entries
            .iter()
            .any(|e| e.root == approval.root && e.agents.iter().any(|a| a.covers(approval)))
    }

    /// Whether the agent is approved in this folder with this executable and
    /// these arguments, for its own configuration or for any provider.
    pub fn is_approved_for_any_provider(&self, approval: &Approval<'_>) -> bool {
        self.entries.iter().any(|e| {
            e.root == approval.root
                && e.agents.iter().any(|a| {
                    a.id == approval.agent
                        && a.program == approval.program
                        && a.args == approval.args
                })
        })
    }

    /// Records the approval, replacing an earlier one for the same agent and
    /// provider in the same folder. An agent can be approved with its own
    /// configuration and with several providers at once.
    pub fn approve(&mut self, approval: &Approval<'_>) -> Result<(), Error> {
        let approved = ApprovedAgent {
            id: approval.agent.to_owned(),
            program: approval.program.to_owned(),
            args: approval.args.to_vec(),
            provider: approval.provider.map(|p| ProviderEntry {
                id: p.id.to_owned(),
                endpoint: p.endpoint.to_owned(),
            }),
            at: now(),
        };
        match self.entries.iter_mut().find(|e| e.root == approval.root) {
            Some(entry) => {
                entry.agents.retain(|a| !a.same_route(approval));
                entry.agents.push(approved);
            }
            None => self.entries.push(ApprovalEntry {
                root: approval.root.to_owned(),
                agents: vec![approved],
            }),
        }
        save(&self.file, &self.entries)
    }

    /// Forgets the agent's approvals in this folder, for every provider.
    pub fn revoke(&mut self, root: &Path, agent: &str) -> Result<(), Error> {
        let before = self.count();
        for entry in &mut self.entries {
            if entry.root == root {
                entry.agents.retain(|a| a.id != agent);
            }
        }
        self.entries.retain(|e| !e.agents.is_empty());
        if self.count() == before {
            return Ok(());
        }
        save(&self.file, &self.entries)
    }

    /// Forgets every approval in this folder, as when its trust is removed.
    pub fn revoke_all(&mut self, root: &Path) -> Result<(), Error> {
        let before = self.entries.len();
        self.entries.retain(|e| e.root != root);
        if self.entries.len() == before {
            return Ok(());
        }
        save(&self.file, &self.entries)
    }

    fn count(&self) -> usize {
        self.entries.iter().map(|e| e.agents.len()).sum()
    }
}

/// Reads a store. A missing file is empty. A damaged or unexpected one is moved
/// aside to `<name>.corrupt` (so nothing is silently destroyed) and treated as
/// empty; the returned warning says so. Entries whose root is not an absolute
/// path are dropped.
pub(crate) fn load<T: DeserializeOwned + Rooted>(file: &Path) -> (Vec<T>, Option<String>) {
    let parsed = match fs::metadata(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), None),
        Err(e) => Err(e.to_string()),
        Ok(metadata) if metadata.len() > MAX_STORE_BYTES => {
            Err(format!("{} bytes is too large", metadata.len()))
        }
        Ok(_) => fs::read(file)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                serde_json::from_slice::<StoreFile<T>>(&bytes).map_err(|e| e.to_string())
            })
            .and_then(|store| {
                if store.version == STORE_VERSION {
                    Ok(store.workspaces)
                } else {
                    Err(format!("unknown version {}", store.version))
                }
            }),
    };
    match parsed {
        Ok(mut entries) => {
            entries.retain(|e| e.root().is_none_or(Path::is_absolute));
            (entries, None)
        }
        Err(reason) => {
            let aside = file.with_extension("json.corrupt");
            let moved = fs::rename(file, &aside).is_ok();
            let warning = format!(
                "{} could not be read ({reason}); starting empty{}",
                file.display(),
                if moved {
                    format!(", the old file is at {}", aside.display())
                } else {
                    String::new()
                }
            );
            (Vec::new(), Some(warning))
        }
    }
}

pub(crate) fn save<T: Serialize + Clone>(file: &Path, entries: &[T]) -> Result<(), Error> {
    let shown = file.display().to_string();
    let io = |e: std::io::Error| Error::io(&shown, e);
    let json = serde_json::to_vec_pretty(&StoreFile {
        version: STORE_VERSION,
        workspaces: entries.to_vec(),
    })
    .map_err(|e| Error::Io {
        path: shown.clone(),
        detail: e.to_string(),
    })?;

    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(io)?;
        // The data directory holds only the user's own settings.
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io)?;
    }
    let temp = file.with_extension(format!("json.tmp-{}", std::process::id()));
    let written = (|| {
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        out.write_all(&json)?;
        out.sync_all()?;
        fs::rename(&temp, file)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written.map_err(io)
}

pub(crate) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
