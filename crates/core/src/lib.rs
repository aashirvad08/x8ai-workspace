//! Contracts shared by every x8ai subsystem.
//!
//! This crate defines the shapes that cross boundaries:
//!
//! - the IPC contract between the webview and the native host ([`app`], [`error`],
//!   [`terminal`], [`workspace`]);
//! - declarative definitions for integrations: coding agents ([`agent`]), model
//!   providers ([`model`]) and MCP servers ([`mcp`]), unified in [`definition`].
//!
//! It deliberately contains no Tauri code, no I/O and no process management, so it
//! builds and tests on any platform. Implementations (PTY sessions, agent runtime,
//! secret store, MCP management) live in their own crates in later phases and depend
//! on this one, never the other way round.
//!
//! Types marked `#[ts(export)]` are written to `src/contracts/generated/` when
//! `cargo test` runs. See `docs/decisions/0003-rust-owned-ipc-contracts.md`.

#![forbid(unsafe_code)]

pub mod agent;
pub mod app;
pub mod definition;
pub mod error;
pub mod id;
pub mod launch;
pub mod mcp;
pub mod model;
pub mod terminal;
pub mod workspace;
