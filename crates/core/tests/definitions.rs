//! Checks that the definition contracts can describe real, diverse integrations
//! without special-casing any of them.
//!
//! `fixtures/definitions.json` is test data, not a shipped catalog. Details such as
//! package versions must be re-verified when the catalog is built (Phase 8).

use std::collections::BTreeSet;

use x8ai_core::agent::AgentDefinition;
use x8ai_core::definition::IntegrationDefinition;
use x8ai_core::model::ModelProviderDefinition;

const FIXTURES: &str = include_str!("fixtures/definitions.json");

fn fixtures() -> Vec<IntegrationDefinition> {
    serde_json::from_str(FIXTURES).expect("fixtures must parse")
}

fn agents(defs: &[IntegrationDefinition]) -> Vec<&AgentDefinition> {
    defs.iter()
        .filter_map(|d| match d {
            IntegrationDefinition::Agent(a) => Some(a),
            _ => None,
        })
        .collect()
}

fn providers(defs: &[IntegrationDefinition]) -> Vec<&ModelProviderDefinition> {
    defs.iter()
        .filter_map(|d| match d {
            IntegrationDefinition::ModelProvider(p) => Some(p),
            _ => None,
        })
        .collect()
}

fn compatible(agent: &AgentDefinition, provider: &ModelProviderDefinition) -> bool {
    provider
        .endpoints
        .iter()
        .any(|e| agent.capabilities.model_apis.contains(&e.api))
}

#[test]
fn every_fixture_parses_and_validates() {
    let defs = fixtures();
    assert!(!defs.is_empty());
    for def in &defs {
        def.validate()
            .unwrap_or_else(|e| panic!("{} failed validation: {e}", def.id()));
    }
}

#[test]
fn ids_are_unique() {
    let defs = fixtures();
    let ids: BTreeSet<_> = defs.iter().map(|d| d.id().as_str()).collect();
    assert_eq!(ids.len(), defs.len());
}

#[test]
fn every_agent_can_use_some_provider() {
    let defs = fixtures();
    for agent in agents(&defs) {
        assert!(
            providers(&defs).iter().any(|p| compatible(agent, p)),
            "{} has no compatible provider",
            agent.id
        );
    }
}

#[test]
fn a_local_provider_serves_agents_from_different_vendors() {
    let defs = fixtures();
    let ollama = providers(&defs)
        .into_iter()
        .find(|p| p.id.as_str() == "ollama")
        .unwrap();
    let served: BTreeSet<_> = agents(&defs)
        .into_iter()
        .filter(|a| compatible(a, ollama))
        .map(|a| a.id.as_str())
        .collect();
    assert!(served.contains("claude-code"));
    assert!(served.contains("opencode"));
    assert!(served.contains("aider"));
}

#[test]
fn round_trips_through_json() {
    for def in fixtures() {
        let json = serde_json::to_string(&def).unwrap();
        let back: IntegrationDefinition = serde_json::from_str(&json).unwrap();
        assert_eq!(back, def);
    }
}

#[test]
fn unknown_fields_are_rejected() {
    // A typo must not silently drop a setting, especially a security-relevant one.
    let json = r#"{
        "kind": "agent", "id": "x", "name": "X",
        "launch": { "program": "x", "sandboxed": true },
        "capabilities": {}
    }"#;
    assert!(serde_json::from_str::<IntegrationDefinition>(json).is_err());
}

#[test]
fn invalid_ids_are_rejected_at_parse_time() {
    let json = r#"{ "kind": "agent", "id": "../../bin", "name": "X",
                    "launch": { "program": "x" }, "capabilities": {} }"#;
    assert!(serde_json::from_str::<IntegrationDefinition>(json).is_err());
}

#[test]
fn api_keys_are_never_sent_over_plain_http_to_remote_hosts() {
    let json = r#"{
        "kind": "modelProvider", "id": "sketchy", "name": "Sketchy",
        "endpoints": [{ "api": "openAiChatCompletions", "baseUrl": "http://models.example.com/v1" }],
        "auth": { "kind": "apiKey", "secret": "SKETCHY_KEY" }
    }"#;
    let def: IntegrationDefinition = serde_json::from_str(json).unwrap();
    let err = def.validate().unwrap_err();
    assert_eq!(err.field, "endpoints[0].baseUrl");
}
