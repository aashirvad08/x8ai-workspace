//! The catalog's own metadata: how an item is presented, never how it runs.

use std::collections::BTreeSet;

use serde::Deserialize;
use x8ai_core::catalog::CatalogSource;

/// Presentation for one item, matched to it by id.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EntryMetadata {
    pub id: String,
    /// Only when known.
    #[serde(default)]
    pub publisher: Option<String>,
    /// The version of this metadata.
    pub version: String,
    pub source: CatalogSource,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    #[error("catalog metadata is not valid: {0}")]
    Parse(String),
    #[error(
        "catalog id {0:?} is not valid: it must be `<type>.<name>`, with type agent, provider, model, mcp or skill"
    )]
    InvalidId(String),
    #[error("catalog id {0} is listed more than once")]
    Duplicate(String),
    #[error("{0}: remote catalog sources are not supported")]
    Remote(String),
    #[error("{id}: {problem}")]
    Invalid { id: String, problem: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Metadata {
    entries: Vec<EntryMetadata>,
}

/// `agent.claude-code`, `model.openrouter.anthropic/claude-sonnet-5`: a type, a
/// dot, then a name without spaces or control characters.
pub fn is_catalog_id(id: &str) -> bool {
    let Some((kind, name)) = id.split_once('.') else {
        return false;
    };
    ["agent", "provider", "model", "mcp", "skill"].contains(&kind)
        && !name.is_empty()
        && id.len() <= 300
        && name.chars().all(|c| c.is_ascii_graphic())
}

impl Metadata {
    /// The metadata that ships with the app.
    pub fn builtin() -> Self {
        Self::parse(include_str!("builtin.json")).expect("builtin.json is checked by tests")
    }

    /// Reads and checks metadata: valid ids, each once, known sources only (never
    /// remote), plain text.
    pub fn parse(json: &str) -> Result<Self, Error> {
        let entries: Vec<EntryMetadata> =
            serde_json::from_str(json).map_err(|e| Error::Parse(e.to_string()))?;
        let mut seen = BTreeSet::new();
        for entry in &entries {
            if !is_catalog_id(&entry.id) {
                return Err(Error::InvalidId(entry.id.clone()));
            }
            if !seen.insert(entry.id.as_str()) {
                return Err(Error::Duplicate(entry.id.clone()));
            }
            if entry.source == CatalogSource::Remote {
                return Err(Error::Remote(entry.id.clone()));
            }
            let text_ok = |t: &str| {
                !t.trim().is_empty() && t.len() <= 100 && !t.chars().any(char::is_control)
            };
            let problem = if !text_ok(&entry.version) {
                Some("the version must be short plain text")
            } else if entry.publisher.as_deref().is_some_and(|p| !text_ok(p)) {
                Some("the publisher must be short plain text")
            } else if entry.tags.len() > 20 || !entry.tags.iter().all(|t| text_ok(t)) {
                Some("at most 20 tags, each short plain text")
            } else {
                None
            };
            if let Some(problem) = problem {
                return Err(Error::Invalid {
                    id: entry.id.clone(),
                    problem: problem.to_owned(),
                });
            }
        }
        Ok(Self { entries })
    }

    pub fn get(&self, id: &str) -> Option<&EntryMetadata> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn entries(&self) -> &[EntryMetadata] {
        &self.entries
    }
}
