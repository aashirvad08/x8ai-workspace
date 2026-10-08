//! Workspaces: a directory the user chose, and scoped file operations inside it.
//!
//! This crate is the filesystem boundary for the webview. Everything the editor,
//! file explorer and search do on disk goes through [`Workspace`], which can reach
//! only the workspace root and what lies beneath it. It also keeps what is
//! remembered between launches: recent workspaces, trust, agent approvals, and
//! spaces with their ids and add-ons ([`RecentWorkspaces`], [`TrustStore`],
//! [`ApprovalStore`], [`SpaceStore`]). No Tauri dependency.

#![forbid(unsafe_code)]

mod error;
mod files;
mod path;
mod search;
mod spaces;
mod store;
mod watch;
mod workspace;

pub use error::{ConflictReason, Error};
pub use search::SearchLimits;
pub use spaces::{
    NEW_SPACE_NAME_RULE, NEW_SPACES_FOLDER, Space, SpaceStore, is_space_id, new_space_name,
};
pub use store::{
    Approval, ApprovalStore, ApprovedProvider, MAX_RECENT, RecentWorkspaces, TrustStore,
};
pub use watch::Watcher;
pub use workspace::{MAX_TEXT_FILE_BYTES, Removal, Workspace};
