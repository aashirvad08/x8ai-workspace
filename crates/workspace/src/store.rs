//! What the app remembers about workspaces between launches: which folders were
//! opened recently, and which the user trusts. Nothing else. No file names, no
//! contents, no settings from inside the folder.
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

#[derive(Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    workspaces: Vec<Entry>,
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

/// Reads a store. A missing file is empty. A damaged or unexpected one is moved
/// aside to `<name>.corrupt` (so nothing is silently destroyed) and treated as
/// empty; the returned warning says so. Entries that are not absolute paths are
/// dropped.
fn load(file: &Path) -> (Vec<Entry>, Option<String>) {
    let parsed = match fs::metadata(file) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Vec::new(), None),
        Err(e) => Err(e.to_string()),
        Ok(metadata) if metadata.len() > MAX_STORE_BYTES => {
            Err(format!("{} bytes is too large", metadata.len()))
        }
        Ok(_) => fs::read(file)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                serde_json::from_slice::<StoreFile>(&bytes).map_err(|e| e.to_string())
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
            entries.retain(|e| e.root.is_absolute());
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

fn save(file: &Path, entries: &[Entry]) -> Result<(), Error> {
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

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}
