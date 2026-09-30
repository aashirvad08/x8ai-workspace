//! Model providers and their models (docs/models.md, ADR 0016).
//!
//! A provider is data: who serves the models, over which APIs, and how it
//! authenticates (an API key held in the Keychain, or nothing for a local server).
//! Which models exist comes from three places, never guessed: the provider's
//! definition (only ids known to be real), models found on this machine (Ollama),
//! and ids the user added. Nothing here knows about any agent; turning a provider
//! and a model into an agent's configuration is the agent adapter's job
//! (`x8ai-agents`).
//!
//! No network access beyond localhost: the app makes no request to a hosted
//! provider on its own. No Tauri dependency.

#![forbid(unsafe_code)]

mod builtin;
pub mod ollama;
pub mod settings;

pub use builtin::builtin;
pub use settings::{ProviderSettings, Settings};

use x8ai_core::model::{ModelDefinition, ModelProviderDefinition, ModelSource};

/// A provider as the webview and the catalog see it: whether it has what it
/// needs to authenticate (never the key), what `local` detection found last
/// (`None`: not checked, and not checked here), and its models.
pub fn status(
    provider: &ModelProviderDefinition,
    credential: x8ai_core::model::CredentialState,
    local: Option<&ollama::Detection>,
    custom: &[String],
) -> x8ai_core::model::ProviderStatus {
    let detection = local.filter(|_| provider.hosting == x8ai_core::model::ProviderKind::Local);
    let found = detection.map(|d| d.models.as_slice()).unwrap_or_default();
    x8ai_core::model::ProviderStatus {
        id: provider.id.clone(),
        name: provider.name.clone(),
        description: provider.description.clone(),
        hosting: provider.hosting,
        credential,
        local: detection.map(|d| d.availability.clone()),
        models: models(provider, found, custom),
    }
}

/// Every model the app can offer for `provider`: its known models, then models
/// found on this machine, then the user's own, without duplicates.
pub fn models(
    provider: &ModelProviderDefinition,
    local: &[String],
    custom: &[String],
) -> Vec<ModelDefinition> {
    let mut models: Vec<ModelDefinition> = provider
        .models
        .iter()
        .map(|m| ModelDefinition {
            id: m.id.clone(),
            provider: provider.id.clone(),
            name: m.name.clone(),
            source: ModelSource::BuiltIn,
            context_window: m.context_window,
        })
        .collect();
    for (ids, source) in [(local, ModelSource::Local), (custom, ModelSource::Custom)] {
        for id in ids {
            if !models.iter().any(|m| &m.id == id) {
                models.push(ModelDefinition {
                    id: id.clone(),
                    provider: provider.id.clone(),
                    name: id.clone(),
                    source,
                    context_window: None,
                });
            }
        }
    }
    models
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_come_from_the_definition_the_machine_and_the_user_without_duplicates() {
        let anthropic = builtin()
            .into_iter()
            .find(|p| p.id.as_str() == "anthropic")
            .unwrap();
        let listed = models(
            &anthropic,
            &[],
            &["claude-sonnet-5".into(), "claude-future-9".into()],
        );
        assert_eq!(
            listed.iter().filter(|m| m.id == "claude-sonnet-5").count(),
            1
        );
        let custom = listed.last().unwrap();
        assert_eq!(
            (custom.id.as_str(), custom.source),
            ("claude-future-9", ModelSource::Custom)
        );
        assert!(listed.iter().all(|m| m.provider == anthropic.id));

        let ollama = builtin()
            .into_iter()
            .find(|p| p.id.as_str() == "ollama")
            .unwrap();
        let listed = models(
            &ollama,
            &["qwen3:8b".into()],
            &["qwen3:8b".into(), "mine:1".into()],
        );
        assert_eq!(
            listed
                .iter()
                .map(|m| (m.id.as_str(), m.source))
                .collect::<Vec<_>>(),
            [
                ("qwen3:8b", ModelSource::Local),
                ("mine:1", ModelSource::Custom)
            ]
        );
    }
}
