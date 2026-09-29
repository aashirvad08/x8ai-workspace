//! Claude Code, configured only through its documented environment variables and
//! `--model` flag (code.claude.com/docs/en/env-vars, /model-config, /llm-gateway;
//! OpenRouter's and Ollama's Claude Code guides). Its settings files and `~/.claude`
//! are never written.
//!
//! - Anthropic: `ANTHROPIC_API_KEY` (sent as `X-Api-Key`). Claude Code's own
//!   endpoint, `api.anthropic.com`, is the provider's. In an interactive session
//!   Claude Code asks once whether to use the key instead of a subscription the
//!   user is logged in with; that question is Claude Code's.
//! - OpenRouter and Ollama serve the Anthropic Messages API: `ANTHROPIC_BASE_URL`
//!   is their endpoint, `ANTHROPIC_AUTH_TOKEN` the key (sent as a bearer token;
//!   Ollama takes the documented placeholder `ollama`), and `ANTHROPIC_API_KEY` is
//!   set empty, as both guides require. The model aliases Claude Code uses on its
//!   own (`opus`, `sonnet`, `haiku`, `fable`, for background work and subagents)
//!   are pointed at the chosen model, so every request goes to it.
//! - Always `CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST=1`, documented for hosts that
//!   manage Claude Code's provider: provider, endpoint and credential variables in
//!   Claude Code's settings files are then ignored, so a settings file cannot send
//!   the session, or the key, elsewhere.
//! - `--model <id>` for the model; it takes precedence over `ANTHROPIC_MODEL` and
//!   the settings files for the session.
//!
//! OpenAI and Google are not supported: Claude Code speaks only the Anthropic
//! Messages API.

use x8ai_core::model::{ModelProviderDefinition, ProviderApi, ProviderEndpoint};
use x8ai_secrets::SecretValue;

use super::{AgentAdapter, Configuration, endpoint_for};

pub struct ClaudeCode;

/// Claude Code variables outside `ANTHROPIC_*` that choose its provider, its
/// credentials or its models (all `ANTHROPIC_*` variables do).
const CONTROLLED: &[&str] = &[
    "AWS_BEARER_TOKEN_BEDROCK",
    "CLAUDE_CODE_OAUTH_REFRESH_TOKEN",
    "CLAUDE_CODE_OAUTH_SCOPES",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST",
    "CLAUDE_CODE_SKIP_ANTHROPIC_AWS_AUTH",
    "CLAUDE_CODE_SKIP_BEDROCK_AUTH",
    "CLAUDE_CODE_SKIP_FOUNDRY_AUTH",
    "CLAUDE_CODE_SKIP_MANTLE_AUTH",
    "CLAUDE_CODE_SKIP_VERTEX_AUTH",
    "CLAUDE_CODE_SUBAGENT_MODEL",
    "CLAUDE_CODE_SUBAGENT_MODEL_FORCE",
    "CLAUDE_CODE_USE_ANTHROPIC_AWS",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_FOUNDRY",
    "CLAUDE_CODE_USE_MANTLE",
    "CLAUDE_CODE_USE_VERTEX",
];

/// The model aliases Claude Code resolves on its own.
const ALIASES: &[&str] = &[
    "ANTHROPIC_DEFAULT_OPUS_MODEL",
    "ANTHROPIC_DEFAULT_SONNET_MODEL",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL",
    "ANTHROPIC_DEFAULT_FABLE_MODEL",
];

/// How Claude Code authenticates to each provider it can use.
enum Auth {
    /// Anthropic's own API: the key as `ANTHROPIC_API_KEY`.
    ApiKey,
    /// An Anthropic-compatible gateway: the key as a bearer token.
    Bearer,
    /// A local server that takes no key but expects this token.
    Placeholder(&'static str),
}

fn auth(provider: &str) -> Option<Auth> {
    match provider {
        "anthropic" => Some(Auth::ApiKey),
        "openrouter" => Some(Auth::Bearer),
        "ollama" => Some(Auth::Placeholder("ollama")),
        _ => None,
    }
}

impl AgentAdapter for ClaudeCode {
    fn agent(&self) -> &'static str {
        "claude-code"
    }

    fn endpoint<'p>(
        &self,
        provider: &'p ModelProviderDefinition,
    ) -> Result<&'p ProviderEndpoint, String> {
        let endpoint = endpoint_for(provider, &[ProviderApi::AnthropicMessages]).ok_or_else(|| {
            "Claude Code speaks only the Anthropic Messages API, which this provider does not serve"
                .to_owned()
        })?;
        auth(provider.id.as_str()).map(|_| endpoint).ok_or_else(|| {
            "the app does not know how to configure Claude Code for this provider".to_owned()
        })
    }

    fn controls(&self, variable: &str) -> bool {
        variable.starts_with("ANTHROPIC_") || CONTROLLED.contains(&variable)
    }

    fn configure(
        &self,
        provider: &ModelProviderDefinition,
        endpoint: &ProviderEndpoint,
        model: &str,
        credential: Option<&SecretValue>,
    ) -> Configuration {
        let set = |name: &str, value: &str| (name.to_owned(), value.to_owned());
        let key = credential.map_or("", SecretValue::expose);
        let mut env = vec![set("CLAUDE_CODE_PROVIDER_MANAGED_BY_HOST", "1")];
        // `endpoint` was chosen by `endpoint`, so the provider is one of these.
        match auth(provider.id.as_str()).expect("a supported provider") {
            Auth::ApiKey => env.push(set("ANTHROPIC_API_KEY", key)),
            auth => {
                let token = match auth {
                    Auth::Placeholder(token) => token,
                    _ => key,
                };
                env.push(set("ANTHROPIC_BASE_URL", &endpoint.base_url));
                env.push(set("ANTHROPIC_AUTH_TOKEN", token));
                env.push(set("ANTHROPIC_API_KEY", ""));
                env.extend(ALIASES.iter().map(|alias| set(alias, model)));
            }
        }
        Configuration {
            env,
            args: vec!["--model".to_owned(), model.to_owned()],
            endpoint: endpoint.base_url.clone(),
        }
    }
}
