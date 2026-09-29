//! The agents the app knows about. Data, not code: adding an agent that needs no
//! special configuration is adding an entry to `builtin.json`.

use x8ai_core::agent::AgentDefinition;

/// Built-in agent definitions, in the order they are shown.
pub fn builtin() -> Vec<AgentDefinition> {
    serde_json::from_str(include_str!("builtin.json")).expect("builtin.json is checked by tests")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn every_builtin_definition_is_valid_and_unique() {
        let agents = builtin();
        assert!(!agents.is_empty());
        for agent in &agents {
            agent
                .validate()
                .unwrap_or_else(|e| panic!("{}: {e}", agent.id));
        }
        let ids: BTreeSet<_> = agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids.len(), agents.len());
    }

    #[test]
    fn no_builtin_agent_bypasses_its_own_approvals_or_needs_a_secret() {
        // docs/security.md, rule 6: never add approval-bypass flags by default.
        for agent in builtin() {
            for arg in &agent.launch.args {
                let arg = arg.to_ascii_lowercase();
                assert!(
                    ![
                        "skip-permissions",
                        "yolo",
                        "auto-approve",
                        "full-auto",
                        "bypass"
                    ]
                    .iter()
                    .any(|flag| arg.contains(flag)),
                    "{} launches with {arg}",
                    agent.id
                );
            }
            // Secrets arrive in Phase 5; until then no definition may need one.
            assert!(
                agent.launch.env.is_empty(),
                "{} sets environment variables",
                agent.id
            );
        }
    }
}
