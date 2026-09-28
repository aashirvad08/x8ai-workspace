//! Workspace contracts.
//!
//! A workspace is a directory the user chose through a native folder picker. The
//! webview refers to everything inside it by *workspace path*: a `/`-separated path
//! relative to the root, such as `src/main.rs`. The empty string is the root. The
//! webview never sees or sends absolute paths for file operations.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The open workspace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct WorkspaceInfo {
    /// Absolute path of the root, for display.
    pub root: String,
    /// The root directory's name.
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EntryKind {
    File,
    Directory,
    /// Sockets, devices, FIFOs, and symlinks that are broken or lead outside the
    /// workspace.
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct DirEntry {
    pub name: String,
    /// Workspace path of the entry.
    pub path: String,
    /// For a symlink, the kind of its target.
    pub kind: EntryKind,
    pub symlink: bool,
}

/// Identifies one version of a file on disk (its modification time and size).
/// Opaque: compare for equality and send back, never parse.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct FileVersion(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileContent {
    pub text: String,
    pub version: FileVersion,
}

/// Files for quick open, gathered on demand. Nothing is indexed or stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct FileList {
    pub paths: Vec<String>,
    /// The walk stopped at its limit, so `paths` is incomplete.
    pub truncated: bool,
}

/// Changes observed on disk, delivered on the channel given to `workspace_open`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum WorkspaceEvent {
    /// These workspace paths were created, modified, removed or renamed.
    Changed { paths: Vec<String> },
    /// Events were dropped. Anything shown may be stale, so reload it.
    Rescan,
}
