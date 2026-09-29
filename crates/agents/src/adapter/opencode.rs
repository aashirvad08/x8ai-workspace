//! OpenCode, configured only through its documented mechanisms
//! (opencode.ai/docs/config, /providers): the provider's API key in the
//! environment variable OpenCode reads for it, and the model and provider options
//! in `OPENCODE_CONFIG_CONTENT`, the inline configuration that takes precedence
//! over the global and the project's `opencode.json`. OpenCode's own files are
//! never written.
//!
//! The inline configuration also pins the provider's endpoint (`baseURL`, in the
//! form the AI SDK packages OpenCode uses expect), so a project's `opencode.json`
//! cannot send the key the app supplies to another host. Ollama has no built-in
//! OpenCode provider; it is declared inline as OpenCode's documentation shows
//! (`@ai-sdk/openai-compatible`, which OpenCode fetches itself on first use).
//!
//! Not verified against a running OpenCode yet (docs/models.md).

use serde_json::json;
use x8ai_core::model::{ModelProviderDefinition, ProviderApi, ProviderEndpoint};
use x8ai_secrets::SecretValue;

use super::{AgentAdapter, Configuration, endpoint_for};

pub struct OpenCode;

/// OpenCode's provider id, the variable it reads the key from, and the APIs of
/// ours its provider package speaks, per provider of ours.
struct Route {
    id: &'static str,
    key: Option<&'static str>,
    apis: &'static [ProviderApi],
}

fn route(provider: &str) -> Option<Route> {
    let route = |id, key, apis| Some(Route { id, key, apis });
    match provider {
        "anthropic" => route(
            "anthropic",
            Some("ANTHROPIC_API_KEY"),
            &[ProviderApi::AnthropicMessages],
        ),
        "openai" => route(
            "openai",
            Some("OPENAI_API_KEY"),
            &[ProviderApi::OpenAiResponses],
        ),
        "google" => route(
            "google",
            Some("GOOGLE_GENERATIVE_AI_API_KEY"),
            &[ProviderApi::Gemini],
        ),
        "openrouter" => route(
            "openrouter",
            Some("OPENROUTER_API_KEY"),
            &[ProviderApi::OpenAiChatCompletions],
        ),
        "ollama" => route("ollama", None, &[ProviderApi::OpenAiChatCompletions]),
        _ => None,
    }
}

const CONTROLLED: &[&str] = &[
    "OPENCODE_CONFIG_CONTENT",
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "GOOGLE_GENERATIVE_AI_API_KEY",
    "OPENROUTER_API_KEY",
];

/// The `baseURL` OpenCode's provider package takes for `endpoint`: the Anthropic
/// package expects the versioned prefix (`…/v1`); the others take the endpoint as
/// it is.
fn base_url(endpoint: &ProviderEndpoint) -> String {
    let base = endpoint.base_url.trim_end_matches('/');
    match endpoint.api {
        ProviderApi::AnthropicMessages => format!("{base}/v1"),
        _ => base.to_owned(),
    }
}

impl AgentAdapter for OpenCode {
    fn agent(&self) -> &'static str {
        "opencode"
    }

    fn endpoint<'p>(
        &self,
        provider: &'p ModelProviderDefinition,
    ) -> Result<&'p ProviderEndpoint, String> {
        let route = route(provider.id.as_str()).ok_or_else(|| {
            "the app does not know how to configure OpenCode for this provider".to_owned()
        })?;
        endpoint_for(provider, route.apis).ok_or_else(|| {
            "this provider does not serve an API OpenCode's provider for it uses".to_owned()
        })
    }

    fn controls(&self, variable: &str) -> bool {
        CONTROLLED.contains(&variable)
    }

    fn configure(
        &self,
        provider: &ModelProviderDefinition,
        endpoint: &ProviderEndpoint,
        model: &str,
        credential: Option<&SecretValue>,
    ) -> Configuration {
        // `endpoint` was chosen by `endpoint`, so the route exists.
        let route = route(provider.id.as_str()).expect("a supported provider");
        let base = base_url(endpoint);
        let mut options = json!({ "options": { "baseURL": base } });
        if route.key.is_none() {
            options["npm"] = json!("@ai-sdk/openai-compatible");
            options["name"] = json!(provider.name);
            options["models"] = json!({ model: { "name": model } });
        }
        let config = json!({
            "$schema": "https://opencode.ai/config.json",
            "model": format!("{}/{model}", route.id),
            "provider": { route.id: options },
        });
        let mut env = vec![("OPENCODE_CONFIG_CONTENT".to_owned(), config.to_string())];
        if let (Some(variable), Some(credential)) = (route.key, credential) {
            env.push((variable.to_owned(), credential.expose().to_owned()));
        }
        Configuration {
            env,
            args: Vec::new(),
            endpoint: base,
        }
    }
}
