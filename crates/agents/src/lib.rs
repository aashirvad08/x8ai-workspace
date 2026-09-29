//! The agent runtime: runs external coding agents (Claude Code, OpenCode, …) in a
//! workspace, safely. See `docs/agent-runtime.md`.
//!
//! The app does not implement an agent. An agent is an external program described
//! by an [`AgentDefinition`](x8ai_core::agent::AgentDefinition); this crate finds
//! it on the user's `PATH` ([`discovery`]), with the environment of the user's
//! login shell ([`environment`]), and starts it on a PTY session in the workspace
//! once the workspace is trusted and the user approved the agent there
//! ([`runtime`]). Nothing here is specific to one agent. No Tauri dependency.

#![forbid(unsafe_code)]

mod builtin;
pub mod discovery;
pub mod environment;
pub mod runtime;

pub use builtin::builtin;
pub use runtime::{
    AgentRuntime, AgentSession, Authorized, Denied, LaunchPlan, RunState, authorize, plan,
};
