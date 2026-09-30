//! What the webview, and the catalog, know about an agent: the runtime's own
//! answer, from its definition, the user's `PATH` and its adapter.

use std::path::Path;

use x8ai_core::agent::{
    AgentAvailability, AgentDefinition, AgentStatus, FeatureSupport, ProviderSupport,
};
use x8ai_core::model::ModelProviderDefinition;

use crate::adapter;
use crate::runtime::{Denied, LaunchPlan, plan};

/// The agent's status: installed when the runtime finds its program on the
/// login `environment`'s `PATH` (nothing is run), what its adapter supports for
/// each of `providers`, for MCP and for skills, and `approved` for its launch.
pub fn status(
    definition: &AgentDefinition,
    environment: &[(String, String)],
    lookup_in: &Path,
    providers: &[ModelProviderDefinition],
    approved: impl Fn(&LaunchPlan) -> bool,
) -> AgentStatus {
    let planned = plan(definition, environment, lookup_in);
    let is_approved = planned.as_ref().is_ok_and(&approved);
    let availability = match planned {
        Ok(plan) => AgentAvailability::Installed {
            executable: plan.program.display().to_string(),
        },
        Err(Denied::NotInstalled { program, .. }) => AgentAvailability::NotInstalled { program },
        Err(_) => AgentAvailability::Unsupported,
    };
    let feature = |support: Result<(), String>| FeatureSupport {
        supported: support.is_ok(),
        reason: support.err(),
    };
    AgentStatus {
        id: definition.id.clone(),
        name: definition.name.clone(),
        description: definition.description.clone(),
        availability,
        approved: is_approved,
        providers: providers
            .iter()
            .map(|provider| {
                let support = adapter::support(definition.id.as_str(), provider);
                ProviderSupport {
                    provider: provider.id.clone(),
                    supported: support.is_ok(),
                    reason: support.err(),
                }
            })
            .collect(),
        mcp: feature(adapter::mcp_support(
            definition.id.as_str(),
            &definition.capabilities.mcp_transports,
        )),
        skills: feature(adapter::skills_support(definition.id.as_str())),
        capabilities: definition.capabilities.clone(),
    }
}
