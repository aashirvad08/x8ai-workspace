//! The unit the catalog will distribute and the user will install or configure.
//!
//! An [`IntegrationDefinition`] is pure data. Loading definitions from disk or a
//! remote catalog, recording where they came from, pinning versions and tracking
//! trust are catalog concerns (Phase 8). Skills and templates become variants here
//! once their schemas are designed (Phase 11).

use std::net::IpAddr;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use url::{Host, Url};

use crate::agent::AgentDefinition;
use crate::id::IntegrationId;
use crate::mcp::McpServerDefinition;
use crate::model::ModelProviderDefinition;

/// Serialized with a `kind` tag: `{ "kind": "agent", "id": "...", ... }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase")]
#[ts(export)]
pub enum IntegrationDefinition {
    Agent(AgentDefinition),
    ModelProvider(ModelProviderDefinition),
    McpServer(McpServerDefinition),
}

impl IntegrationDefinition {
    pub fn id(&self) -> &IntegrationId {
        match self {
            Self::Agent(d) => &d.id,
            Self::ModelProvider(d) => &d.id,
            Self::McpServer(d) => &d.id,
        }
    }

    /// Checks the rules serde cannot express. Identifier rules are already enforced
    /// during deserialization.
    pub fn validate(&self) -> Result<(), DefinitionError> {
        match self {
            Self::Agent(d) => d.validate(),
            Self::ModelProvider(d) => d.validate(),
            Self::McpServer(d) => d.validate(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field}: {reason}")]
pub struct DefinitionError {
    /// Path of the offending field, e.g. `launch.env[2].name`.
    pub field: String,
    pub reason: String,
}

impl DefinitionError {
    pub fn new(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            reason: reason.into(),
        }
    }
}

pub(crate) fn check_name(name: &str) -> Result<(), DefinitionError> {
    if name.trim().is_empty() {
        return Err(DefinitionError::new("name", "must not be empty"));
    }
    Ok(())
}

/// Endpoints must be `http` or `https`, must not embed credentials, and must use
/// `https` when `sensitive` traffic goes to anything other than the local machine.
pub(crate) fn check_endpoint_url(
    field: &str,
    value: &str,
    sensitive: bool,
) -> Result<(), DefinitionError> {
    let url = Url::parse(value)
        .map_err(|e| DefinitionError::new(field, format!("is not a valid URL: {e}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(DefinitionError::new(field, "must use http or https"));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(DefinitionError::new(
            field,
            "must not embed credentials; reference a secret instead",
        ));
    }
    if sensitive && url.scheme() == "http" && !is_loopback(&url) {
        return Err(DefinitionError::new(
            field,
            "must use https unless the host is the local machine",
        ));
    }
    Ok(())
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_urls_require_http_schemes() {
        assert!(check_endpoint_url("u", "https://api.example.com/v1", true).is_ok());
        assert!(check_endpoint_url("u", "file:///etc/passwd", false).is_err());
        assert!(check_endpoint_url("u", "not a url", false).is_err());
    }

    #[test]
    fn sensitive_traffic_needs_https_off_the_local_machine() {
        assert!(check_endpoint_url("u", "http://api.example.com", true).is_err());
        assert!(check_endpoint_url("u", "http://api.example.com", false).is_ok());
        for local in [
            "http://localhost:11434",
            "http://127.0.0.1:8080",
            "http://[::1]:1234",
        ] {
            assert!(check_endpoint_url("u", local, true).is_ok(), "{local}");
        }
    }

    #[test]
    fn endpoint_urls_must_not_embed_credentials() {
        let err = check_endpoint_url("u", "https://user:hunter2@example.com", false).unwrap_err();
        assert!(err.reason.contains("secret"));
    }
}
