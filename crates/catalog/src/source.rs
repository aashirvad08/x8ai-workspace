//! Where catalog metadata comes from: built into the app, and nothing else yet.
//!
//! These are the seams a later phase can use to add a remote catalog
//! (docs/catalog.md, "Future remote catalogs"):
//!
//! ```text
//! remote catalog → signed metadata → Verifier → verified package → install
//!                → the owning system (agent runtime, provider, MCP or skill registry)
//! ```
//!
//! Only [`Builtin`] exists. No [`Verifier`] is implemented, nothing here fetches,
//! verifies or installs anything, and [`Metadata::parse`] refuses metadata that
//! claims to be remote, so nothing can pass itself off as verified.

use x8ai_core::catalog::CatalogSource;

use crate::{Error, Metadata};

/// A place catalog metadata is read from.
pub trait MetadataSource {
    /// How items described by it are labelled.
    fn kind(&self) -> CatalogSource;

    /// Its metadata, checked. A remote source would return only what a
    /// [`Verifier`] accepted; presentation only, as today, so it still configures
    /// nothing and every item's own system stays its source of truth.
    fn metadata(&self) -> Result<Metadata, Error>;
}

/// The metadata that ships with the app.
#[derive(Debug, Clone, Copy, Default)]
pub struct Builtin;

impl MetadataSource for Builtin {
    fn kind(&self) -> CatalogSource {
        CatalogSource::Builtin
    }

    fn metadata(&self) -> Result<Metadata, Error> {
        Ok(Metadata::builtin())
    }
}

/// Metadata as a remote catalog would deliver it: the bytes, a detached
/// signature over them, and which publisher key signed them. Unused today.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedMetadata {
    pub bytes: Vec<u8>,
    pub signature: Vec<u8>,
    pub key_id: String,
}

/// Checks signed metadata against publisher keys the user chose to trust, and
/// only then parses it. Deliberately unimplemented: the phase that adds remote
/// catalogs also adds key management, and installation stays with the system
/// that owns each kind of item, behind its own approval.
pub trait Verifier {
    fn verify(&self, signed: &SignedMetadata) -> Result<Metadata, Error>;
}
