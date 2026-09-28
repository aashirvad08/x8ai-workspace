//! Workspaces: a directory the user chose, and scoped file operations inside it.
//!
//! This crate is the filesystem boundary for the webview. Everything the editor
//! and file explorer do on disk goes through [`Workspace`], which can reach only
//! the workspace root and what lies beneath it. No Tauri dependency.

#![forbid(unsafe_code)]

mod error;
mod files;
mod path;
mod watch;
mod workspace;

pub use error::{ConflictReason, Error};
pub use watch::Watcher;
pub use workspace::{MAX_TEXT_FILE_BYTES, Removal, Workspace};
