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
    /// The space's id (`ws-k3f9qa`), made when the folder is first opened.
    pub id: String,
    /// Absolute path of the root, for display.
    pub root: String,
    /// The root directory's name.
    pub name: String,
    /// Whether the user has explicitly trusted this folder. New folders are
    /// untrusted. See `docs/decisions/0010-workspace-trust-and-recent-workspaces.md`.
    pub trusted: bool,
}

/// A previously opened workspace. Only its location is remembered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct RecentWorkspace {
    /// The space's id, when it has one (every folder opened since ids exist).
    pub id: Option<String>,
    pub root: String,
    pub name: String,
    /// The folder still exists. Missing folders (an unmounted drive, a deleted
    /// checkout) stay listed until removed or reopened unsuccessfully.
    pub available: bool,
}

/// A plain-text search across the workspace. The text is matched literally.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchQuery {
    pub text: String,
    pub case_sensitive: bool,
}

/// One matching line. Columns are UTF-16 offsets, the unit the editor uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchMatch {
    /// 1-based line number.
    pub line: u32,
    /// Where the first match starts in the line, and its length.
    pub column: u32,
    pub length: u32,
    /// The line, shortened around the match if it is long.
    pub preview: String,
    /// Start and end of every match within `preview`.
    pub ranges: Vec<(u32, u32)>,
}

/// Results of a search, streamed on the channel given to `workspace_search`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum SearchEvent {
    /// Every match in one file.
    File {
        path: String,
        matches: Vec<SearchMatch>,
    },
    /// The search finished. Nothing follows.
    Done(SearchSummary),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SearchSummary {
    pub files: u32,
    pub matches: u32,
    /// A result limit was reached, so there may be more matches.
    pub truncated: bool,
    /// Superseded by a newer search, or cancelled.
    pub cancelled: bool,
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
