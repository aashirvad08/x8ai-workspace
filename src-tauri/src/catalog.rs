//! The catalog command (docs/catalog.md, ADR 0018): every agent, provider,
//! model, MCP server and skill the app knows, as their own systems report them,
//! with the catalog's presentation metadata.
//!
//! Read-only, and it starts nothing. Agents are looked up as the Agents view
//! does (the runtime finds programs on the login `PATH`; nothing runs). Providers
//! report their last local detection; Ollama is never probed from here. MCP
//! servers report their state; none is started. Keys and secrets are reported
//! as saved or not, by their own systems, and never read. Acting on an item goes
//! through the command of the system that owns it.

use tauri::{AppHandle, Manager};
use x8ai_catalog::{Facts, Metadata, assemble};
use x8ai_core::catalog::CatalogList;
use x8ai_core::error::{CommandError, ErrorCode};

use crate::agents::{Agents, agent_statuses, definitions, login_environment};
use crate::mcp::Mcp;
use crate::providers::Providers;
use crate::skills::Skills;
use crate::workspace::Workspaces;

#[tauri::command]
pub async fn catalog_list(app: AppHandle) -> Result<CatalogList, CommandError> {
    let environment = login_environment(&app).await?;
    let task_app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let app = task_app;
        let providers = app.state::<Providers>();
        let definitions = definitions(&app);
        let agents = agent_statuses(
            &app.state::<Agents>(),
            &environment,
            &app.state::<Workspaces>(),
            &providers,
        );
        let provider_statuses = providers.statuses();
        let path = x8ai_agents::environment::var(&environment.vars, "PATH");
        let mcp = app.state::<Mcp>().statuses(path, &definitions);
        let skills = app.state::<Skills>().statuses(&definitions);
        Ok(assemble(
            &Metadata::builtin(),
            &Facts {
                agents: &agents,
                providers: &provider_statuses,
                mcp: &mcp,
                skills: &skills,
            },
        ))
    })
    .await
    .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

#[cfg(test)]
mod tests {
    /// The command above, without this test: it may only read statuses.
    fn command_source() -> &'static str {
        let source = include_str!("catalog.rs");
        &source[..source.find("#[cfg(test)]").unwrap()]
    }

    #[test]
    fn listing_the_catalog_starts_probes_unlocks_and_approves_nothing() {
        let source = command_source();
        for forbidden in [
            "start_mcp",
            "run(",
            "Command::new",
            "spawn(",
            "detect(",
            "refresh",
            "credential(",
            ".secrets",
            "SecretStore",
            "approve(",
            "set_trust",
            "request_approval",
            "reqwest",
            "http",
        ] {
            assert!(
                !source.contains(forbidden),
                "catalog_list contains {forbidden}"
            );
        }
        // It reads what each owning system already reports.
        for owner in [
            "agent_statuses(",
            "providers.statuses()",
            ".statuses(path",
            "statuses(&definitions)",
        ] {
            assert!(source.contains(owner), "catalog_list does not use {owner}");
        }
    }
}
