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
            if model.id.trim().is_empty() {
                return Err(DefinitionError::new(
                    format!("models[{i}].id"),
                    "must not be empty",
                ));
            }
        }
        Ok(())
    }
}
