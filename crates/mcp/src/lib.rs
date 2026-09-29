//! MCP servers for agent sessions (docs/mcp.md, ADR 0017).
//!
//! The app owns the servers the user configured (`registry`), which workspace
//! allowed exactly which of them (`approvals`), which servers a session gets and
//! what exactly each would run (`session`), the environment a server gets
//! (`environment`), and, for stdio servers, the processes themselves, which
//! belong to the agent session they were started for (`runtime`, `bridge`). The
//! agent stays the MCP client: its adapter (`x8ai-agents`) tells it where each
//! server is, for that session only. The app never speaks MCP itself, never
//! contacts an HTTP server, and never installs or downloads anything. No Tauri
//! dependency.

#![forbid(unsafe_code)]

pub mod approvals;
pub mod bridge;
pub mod environment;
mod files;
pub mod registry;
pub mod runtime;
pub mod session;

pub use approvals::{Approvals, Material, MaterialTransport};
pub use environment::{ServerEnvironment, environment, secret_account};
pub use registry::Registry;
pub use runtime::{Endpoint, Launch, Limits, McpRuntime};
pub use session::{
    Authorized, Denied, Prepared, attach, authorize, prepare, still_attached, unapproved,
};
