//! Model providers: Anthropic, OpenAI, Google, OpenRouter, Ollama and others.
//!
//! A provider is described by the wire APIs it serves, not by who operates it.
//! OpenRouter and a local Ollama both serve an OpenAI-compatible Chat Completions
//! API, so every agent that speaks that API can use either. This is what keeps the
//! app from assuming "AI" means one vendor.
//!
//! In the initial design the app does not call models itself: agents call providers
//! directly using configuration and credentials the app supplies at launch. See
//! `docs/architecture.md`, "Model layer".

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::definition::{DefinitionError, check_endpoint_url, check_name};
use crate::id::{IntegrationId, SecretName};

/// A model wire protocol. An agent and a provider are compatible when they share one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ProviderApi {
    /// Anthropic Messages API.
    AnthropicMessages,
    /// OpenAI Chat Completions API, also served by most OpenAI-compatible servers.
    OpenAiChatCompletions,
    /// OpenAI Responses API.
    OpenAiResponses,
    /// Google Gemini API.
    Gemini,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ModelProviderDefinition {
    pub id: IntegrationId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Who runs it: a hosted service, a gateway to other providers, or a server on
    /// this machine. (Not `kind`, which tags the definition type.)
    #[serde(default)]
    pub hosting: ProviderKind,
    /// The APIs this provider serves. At least one.
    pub endpoints: Vec<ProviderEndpoint>,
    pub auth: ProviderAuth,
    /// Models known in advance. Providers with dynamic model lists (Ollama,
    /// OpenRouter) are queried at runtime from Phase 6, so this may be empty.
    #[serde(default)]
    pub models: Vec<ModelInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ProviderEndpoint {
    pub api: ProviderApi,
    pub base_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub enum ProviderAuth {
    /// No credentials, e.g. a local Ollama server.
    None,
    /// An API key held in the native secret store.
    ApiKey { secret: SecretName },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct ModelInfo {
    /// The provider's own model id, e.g. `qwen3-coder:30b`.
    pub id: String,
    pub name: String,
    /// Tokens, only when verified against the provider's documentation. Left out
    /// rather than guessed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ProviderKind {
    /// A provider's own hosted API.
    #[default]
    Hosted,
    /// A service that routes to other providers' models (OpenRouter).
    Gateway,
    /// A server on this machine (Ollama). Nothing leaves the machine.
    Local,
}

impl ModelProviderDefinition {
    pub fn validate(&self) -> Result<(), DefinitionError> {
        check_name(&self.name)?;
        if self.endpoints.is_empty() {
            return Err(DefinitionError::new(
                "endpoints",
                "must list at least one endpoint",
            ));
        }
        let sends_credentials = matches!(self.auth, ProviderAuth::ApiKey { .. });
        for (i, endpoint) in self.endpoints.iter().enumerate() {
            check_endpoint_url(
                &format!("endpoints[{i}].baseUrl"),
                &endpoint.base_url,
                sends_credentials,
            )?;
        }
        for (i, model) in self.models.iter().enumerate() {
            if !is_model_id(&model.id) {
                return Err(DefinitionError::new(
                    format!("models[{i}].id"),
                    MODEL_ID_RULE,
                ));
            }
        }
        Ok(())
    }
}

pub const MODEL_ID_RULE: &str = "must be 1-200 characters of letters, digits and . _ : / @ + - [ ], \
                                  starting with a letter or digit";

/// Model ids as providers write them (`claude-sonnet-5`, `anthropic/claude-sonnet-5`,
/// `qwen3-coder:30b`, `opus[1m]`). Never starting with `-`, so an id can never be
/// taken for a command-line option.
pub fn is_model_id(value: &str) -> bool {
    let mut chars = value.chars();
    value.len() <= 200
        && chars.next().is_some_and(|c| c.is_ascii_alphanumeric())
        && chars.all(|c| c.is_ascii_alphanumeric() || "._:/@+-[]".contains(c))
}

// IPC contracts of the provider layer (docs/models.md). None of them ever carries
// a credential.

/// Everything the webview knows about a provider. Returned by `provider_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderStatus {
    pub id: IntegrationId,
    pub name: String,
    pub description: String,
    pub hosting: ProviderKind,
    pub credential: CredentialState,
    /// For a local provider, whether it is here; `None` for hosted ones, or when
    /// not checked.
    pub local: Option<LocalAvailability>,
    pub models: Vec<ModelDefinition>,
}

/// Whether a provider has what it needs to authenticate. Never the credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum CredentialState {
    /// The provider needs none (a local server).
    NotNeeded,
    Missing,
    /// Saved in the macOS Keychain.
    InKeychain,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export)]
pub enum LocalAvailability {
    /// Neither installed nor answering.
    Unavailable,
    /// Installed, but its server is not answering.
    Installed,
    /// Its server answers.
    Available { version: Option<String> },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelDefinition {
    pub id: String,
    pub provider: IntegrationId,
    pub name: String,
    pub source: ModelSource,
    /// Only when verified; see [`ModelInfo::context_window`].
    pub context_window: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ModelSource {
    /// Listed in the app's provider definition.
    BuiltIn,
    /// Found on this machine (a model Ollama has).
    Local,
    /// Added by the user.
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ProviderList {
    pub providers: Vec<ProviderStatus>,
}

/// A provider and a model for an agent session.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct ModelSelection {
    pub provider: IntegrationId,
    pub model: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_ids_are_what_providers_use_and_never_options() {
        for good in [
            "claude-sonnet-5",
            "anthropic/claude-sonnet-5",
            "qwen3-coder:30b",
            "opus[1m]",
            "gpt-4.1",
            "models/gemini-pro",
        ] {
            assert!(is_model_id(good), "{good}");
        }
        for bad in [
            "",
            "-m",
            "--dangerously-skip-permissions",
            "a b",
            "model;rm",
            "x".repeat(201).as_str(),
            "$(id)",
        ] {
            assert!(!is_model_id(bad), "{bad}");
        }
    }
}
