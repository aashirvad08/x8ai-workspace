//! Codex, configured only through its documented command-line options: `-m` for
//! the model, and `-c key=value` overrides, which apply to this run only and take
//! precedence over `~/.codex/config.toml`. The overrides declare a model
//! provider of the app's (`x8ai`): the provider's endpoint pinned as its
//! `base_url`, the Responses wire API, and as its `env_key` the name of the
//! variable that holds the key, in this session's environment only. Then they
//! select it. Codex's own files are never written, and a provider in the user's
//! or the project's Codex configuration cannot take the key elsewhere.
//!
//! Codex speaks only the OpenAI Responses API (`wire_api = "responses"`; it
//! refuses `"chat"`), so of the app's providers it can use OpenAI.
//!
//! MCP servers and skills are not supported yet.

use x8ai_core::model::{ModelProviderDefinition, ProviderApi, ProviderEndpoint};
use x8ai_secrets::SecretValue;

use super::{AgentAdapter, Configuration, endpoint_for};

pub struct Codex;

/// The provider id the overrides declare. Codex's built-in providers cannot be
/// overridden, so it is the app's own.
const PROVIDER: &str = "x8ai";

/// Where Codex reads the key: a name of the app's, so a key in the user's
/// shell is never used instead of the one chosen for the session.
pub const KEY_VARIABLE: &str = "X8AI_CODEX_API_KEY";

/// Variables of the user's shell that would decide Codex's provider, endpoint or
/// key: removed when the app configures the session (ADR 0015).
const CONTROLLED: &[&str] = &[
    KEY_VARIABLE,
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "CODEX_API_KEY",
    "OPENAI_ORGANIZATION",
    "OPENAI_PROJECT",
];

/// `value` as a TOML basic string. JSON string escapes are valid TOML ones.
fn toml_string(value: &str) -> String {
    serde_json::Value::String(value.to_owned()).to_string()
}

impl AgentAdapter for Codex {
    fn agent(&self) -> &'static str {
        "codex"
    }

    fn endpoint<'p>(
        &self,
        provider: &'p ModelProviderDefinition,
    ) -> Result<&'p ProviderEndpoint, String> {
        endpoint_for(provider, &[ProviderApi::OpenAiResponses]).ok_or_else(|| {
            "Codex speaks only the OpenAI Responses API, which this provider does not serve"
                .to_owned()
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
        let base = endpoint.base_url.trim_end_matches('/').to_owned();
        let set =
            |key: &str, value: &str| ["-c".to_owned(), format!("{key}={}", toml_string(value))];
        let mut args = Vec::new();
        args.extend(set(
            &format!("model_providers.{PROVIDER}.name"),
            &provider.name,
        ));
        args.extend(set(&format!("model_providers.{PROVIDER}.base_url"), &base));
        args.extend(set(
            &format!("model_providers.{PROVIDER}.env_key"),
            KEY_VARIABLE,
        ));
        args.extend(set(
            &format!("model_providers.{PROVIDER}.wire_api"),
            "responses",
        ));
        args.extend(set("model_provider", PROVIDER));
        args.extend(["-m".to_owned(), model.to_owned()]);
        let env = credential
            .map(|key| vec![(KEY_VARIABLE.to_owned(), key.expose().to_owned())])
            .unwrap_or_default();
        Configuration {
            env,
            args,
            endpoint: base,
        }
    }
}
