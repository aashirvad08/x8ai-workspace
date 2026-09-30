//! The agent runtime: runs external coding agents (Claude Code, OpenCode, …) in a
//! workspace, safely. See `docs/agent-runtime.md`.
//!
//! The app does not implement an agent. An agent is an external program described
//! by an [`AgentDefinition`](x8ai_core::agent::AgentDefinition); this crate finds
//! it on the user's `PATH` ([`discovery`]), with the environment of the user's
//! login shell ([`environment`]), and starts it on a PTY session once the
//! workspace is trusted and the user approved the agent there ([`runtime`]). In a
//! Git repository each agent session gets a worktree of its own ([`isolation`]),
//! so several agents can work at once without touching the user's working tree.
//! An agent can be pointed at a provider and a model the user chose in the app
//! ([`adapter`]); that is the only place that knows how a particular agent is
//! configured. No Tauri dependency.

#![forbid(unsafe_code)]

pub mod adapter;
mod builtin;
pub mod discovery;
pub mod environment;
pub mod isolation;
pub mod runtime;
mod status;

pub use builtin::builtin;
pub use isolation::{Isolation, Removal, Worktree};
pub use runtime::{
    AgentRuntime, AgentSession, Authorized, Denied, LaunchPlan, ProviderRoute, RunError,
    SessionState, authorize, plan,
};
pub use status::status;
