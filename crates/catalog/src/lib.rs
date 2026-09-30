//! The catalog (docs/catalog.md, ADR 0018): discovery and orchestration over the
//! agents, models, MCP servers and skills the app knows. Not execution.
//!
//! An item is built from two things: **facts** its owning system reports (the
//! agent runtime's definitions and whether each is installed, the provider
//! registry's providers and models, the MCP registry's servers, the skill
//! registry's skills) and, for built-in items, **presentation metadata** of the
//! catalog's own (publisher, tags, metadata version). The metadata has no field
//! that configures anything, so there is exactly one source of truth for each
//! item: its system. Metadata that matches no item is ignored, never shown as
//! something that exists.
//!
//! This crate is pure data. It depends on the contracts alone. It starts no
//! process, opens no connection, reads no secret, and has no access to trust or
//! approvals (a test checks its dependencies and source). Acting on an item is
//! asking its owning system, which decides. Metadata is built in; the seams for
//! a future signed remote catalog are in [`MetadataSource`] and [`Verifier`],
//! and nothing implements them remotely.

#![forbid(unsafe_code)]

mod assemble;
mod metadata;
mod source;

pub use assemble::{
    Facts, agent_status, assemble, mcp_status, model_status, provider_status, skill_status,
};
pub use metadata::{EntryMetadata, Error, Metadata, is_catalog_id};
pub use source::{Builtin, MetadataSource, SignedMetadata, Verifier};
