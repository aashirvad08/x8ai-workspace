//! The providers the app knows about. Adding one that fits the existing APIs is
//! adding an entry to `builtin.json`.

use x8ai_core::model::ModelProviderDefinition;

/// Built-in provider definitions, in the order they are shown.
pub fn builtin() -> Vec<ModelProviderDefinition> {
    serde_json::from_str(include_str!("builtin.json")).expect("builtin.json is checked by tests")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use x8ai_core::model::{ProviderApi, ProviderAuth, ProviderKind};

    use super::*;

    #[test]
    fn every_builtin_provider_is_valid_and_unique() {
        let providers = builtin();
        let ids: Vec<&str> = providers.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            ["anthropic", "openai", "google", "openrouter", "ollama"]
        );
        assert_eq!(ids.iter().collect::<BTreeSet<_>>().len(), ids.len());
        for provider in &providers {
            provider
                .validate()
                .unwrap_or_else(|e| panic!("{}: {e}", provider.id));
        }
    }

    #[test]
    fn hosted_providers_need_a_key_and_the_local_one_does_not() {
        for provider in builtin() {
            match provider.hosting {
                ProviderKind::Local => {
                    assert_eq!(provider.auth, ProviderAuth::None);
                    assert!(
                        provider
                            .endpoints
                            .iter()
                            .all(|e| e.base_url.starts_with("http://localhost:"))
                    );
                }
                ProviderKind::Hosted | ProviderKind::Gateway => {
                    assert!(
                        matches!(provider.auth, ProviderAuth::ApiKey { .. }),
                        "{}",
                        provider.id
                    );
                    assert!(
                        provider
                            .endpoints
                            .iter()
                            .all(|e| e.base_url.starts_with("https://")),
                        "{}",
                        provider.id
                    );
                }
            }
        }
    }

    #[test]
    fn no_capability_is_claimed_that_was_not_verified() {
        // Context windows and the like are left out rather than guessed.
        for provider in builtin() {
            assert!(
                provider.models.iter().all(|m| m.context_window.is_none()),
                "{}",
                provider.id
            );
        }
    }

    #[test]
    fn the_gateway_and_local_server_speak_the_anthropic_api_too() {
        let speaks = |id: &str| {
            builtin()
                .into_iter()
                .find(|p| p.id.as_str() == id)
                .unwrap()
                .endpoints
                .iter()
                .any(|e| e.api == ProviderApi::AnthropicMessages)
        };
        assert!(speaks("openrouter") && speaks("ollama") && speaks("anthropic"));
        assert!(!speaks("openai") && !speaks("google"));
    }
}
