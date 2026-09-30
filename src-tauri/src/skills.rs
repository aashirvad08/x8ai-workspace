//! Skill commands: the IPC face of `x8ai-skills` (docs/catalog.md).
//!
//! The webview can list skills, and add, change and remove the user's own. Built-in
//! skills are read-only. A skill is text: nothing here runs, reads a secret, or
//! changes a provider, an MCP server, an agent, trust or approvals. Skills reach an
//! agent only through the agent commands, for a session they were chosen for.

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use tauri::{AppHandle, Manager};
use x8ai_agents::adapter;
use x8ai_core::agent::AgentDefinition;
use x8ai_core::error::{CommandError, ErrorCode};
use x8ai_core::mcp::McpAgentSupport;
use x8ai_core::skill::{
    SessionSkill, SessionSkillState, Skill, SkillInput, SkillList, SkillRef, SkillStatus,
};
use x8ai_skills::SkillRegistry;

use crate::workspace::Workspaces;

/// The skill registry. Managed Tauri state.
#[derive(Default)]
pub struct Skills {
    registry: Mutex<Option<SkillRegistry>>,
}

impl Skills {
    /// Loads the user's skills from the app's data directory.
    pub fn load(&self, data_dir: &Path, workspaces: &Workspaces) {
        let (registry, warnings) = SkillRegistry::load(data_dir.join("skills.json"));
        for warning in warnings {
            workspaces.warn(warning);
        }
        *lock(&self.registry) = Some(registry);
    }

    /// Built-in skills, then the user's.
    pub(crate) fn skills(&self) -> Vec<Skill> {
        lock(&self.registry)
            .as_ref()
            .map_or_else(x8ai_skills::builtin, SkillRegistry::skills)
    }

    /// Each skill and which agents can take it, as their adapters say.
    pub(crate) fn statuses(&self, agents: &[AgentDefinition]) -> Vec<SkillStatus> {
        self.skills()
            .into_iter()
            .map(|skill| SkillStatus {
                skill,
                agents: agents
                    .iter()
                    .map(|agent| {
                        let support = adapter::skills_support(agent.id.as_str());
                        McpAgentSupport {
                            agent: agent.id.clone(),
                            supported: support.is_ok(),
                            reason: support.err(),
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    /// A session's skills as recorded, and whether each is still exactly that.
    pub(crate) fn session_skills(&self, recorded: &[SkillRef]) -> Vec<SessionSkill> {
        let skills = self.skills();
        recorded
            .iter()
            .map(|r| match skills.iter().find(|s| s.id == r.id) {
                None => SessionSkill {
                    id: r.id.clone(),
                    name: r.id.to_string(),
                    version: r.version,
                    state: SessionSkillState::Removed,
                },
                Some(skill) => SessionSkill {
                    id: r.id.clone(),
                    name: skill.name.clone(),
                    version: r.version,
                    state: if skill.fingerprint() == r.fingerprint && skill.version == r.version {
                        SessionSkillState::Attached
                    } else {
                        SessionSkillState::Changed {
                            current_version: skill.version,
                        }
                    },
                },
            })
            .collect()
    }

    fn with_registry<T>(
        &self,
        f: impl FnOnce(&mut SkillRegistry) -> Result<T, x8ai_skills::Error>,
    ) -> Result<T, CommandError> {
        let mut registry = lock(&self.registry);
        let registry = registry.as_mut().ok_or_else(|| {
            CommandError::new(ErrorCode::Internal, "the skill registry is unavailable")
        })?;
        f(registry).map_err(skill_error)
    }
}

/// Every skill, built-in and the user's, with which agents can take it.
#[tauri::command]
pub fn skill_list(app: AppHandle) -> SkillList {
    SkillList {
        skills: app
            .state::<Skills>()
            .statuses(&crate::agents::definitions(&app)),
    }
}

/// Adds a user skill. A workspace skill belongs to the open folder.
#[tauri::command]
pub fn skill_add(skill: SkillInput, app: AppHandle) -> Result<Skill, CommandError> {
    let root = app.state::<Workspaces>().root();
    app.state::<Skills>()
        .with_registry(|r| r.add(&skill, root.as_deref()))
}

/// Changes a user skill. Sessions that recorded the old version do not run with
/// the new one: they say so, and a new session takes it.
#[tauri::command]
pub fn skill_update(id: String, skill: SkillInput, app: AppHandle) -> Result<Skill, CommandError> {
    let root = app.state::<Workspaces>().root();
    app.state::<Skills>()
        .with_registry(|r| r.update(&id, &skill, root.as_deref()))
}

/// Removes a user skill. Sessions that have it do not run until started anew.
#[tauri::command]
pub fn skill_remove(id: String, app: AppHandle) -> Result<(), CommandError> {
    app.state::<Skills>()
        .with_registry(|r| r.remove(&id).map(|_| ()))
}

fn skill_error(error: x8ai_skills::Error) -> CommandError {
    use x8ai_skills::Error;
    let code = match &error {
        Error::Invalid(_) | Error::NoWorkspace | Error::Full | Error::NotSessionScoped(_) => {
            ErrorCode::InvalidInput
        }
        Error::NotFound(_) => ErrorCode::NotFound,
        Error::Builtin(_) => ErrorCode::PermissionDenied,
        Error::Io { .. } => ErrorCode::Internal,
    };
    CommandError::new(code, error.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use x8ai_core::mcp::McpScopeKind;

    use super::*;

    fn registry(name: &str) -> (Skills, std::path::PathBuf) {
        let temp =
            std::env::temp_dir().join(format!("x8ai-skills-desktop-{name}-{}", std::process::id()));
        let skills = Skills::default();
        *lock(&skills.registry) = Some(SkillRegistry::load(temp.join("skills.json")).0);
        (skills, temp)
    }

    #[test]
    fn a_session_says_whether_each_skill_is_still_the_one_it_recorded() {
        let (skills, temp) = registry("session");
        let input = SkillInput {
            name: "HEP analysis".into(),
            description: String::new(),
            instructions: "Use ROOT conventions.".into(),
            allowed_tools: Vec::new(),
            scope: McpScopeKind::Session,
        };
        let mine = skills.with_registry(|r| r.add(&input, None)).unwrap();
        let builtin = x8ai_skills::builtin().remove(0);
        let recorded = vec![builtin.reference(), mine.reference()];
        let states = |s: &Skills| {
            s.session_skills(&recorded)
                .into_iter()
                .map(|s| s.state)
                .collect::<Vec<_>>()
        };
        assert_eq!(
            states(&skills),
            [SessionSkillState::Attached, SessionSkillState::Attached]
        );

        let changed = SkillInput {
            instructions: "Use ROOT 6 conventions.".into(),
            ..input
        };
        skills
            .with_registry(|r| r.update(mine.id.as_str(), &changed, None))
            .unwrap();
        assert_eq!(
            states(&skills),
            [
                SessionSkillState::Attached,
                SessionSkillState::Changed { current_version: 2 }
            ]
        );

        skills
            .with_registry(|r| r.remove(mine.id.as_str()))
            .unwrap();
        let listed = skills.session_skills(&recorded);
        assert_eq!(listed[1].state, SessionSkillState::Removed);
        // What it was called is not kept anywhere but the registry: the id stands in.
        assert_eq!(listed[1].name, mine.id.to_string());
        // Built-in skills can be neither changed nor removed.
        assert!(
            skills
                .with_registry(|r| r.remove(builtin.id.as_str()))
                .is_err()
        );
        let _ = std::fs::remove_dir_all(temp);
    }

    #[test]
    fn only_agents_whose_adapter_takes_skills_are_offered_them() {
        let (skills, temp) = registry("support");
        let statuses = skills.statuses(&x8ai_agents::builtin());
        assert!(!statuses.is_empty());
        for status in &statuses {
            let supported = status
                .agents
                .iter()
                .filter(|a| a.supported)
                .map(|a| a.agent.as_str())
                .collect::<Vec<_>>();
            assert_eq!(supported, ["claude-code"], "{}", status.skill.id);
            assert!(
                status
                    .agents
                    .iter()
                    .all(|a| a.supported || a.reason.is_some())
            );
        }
        let _ = std::fs::remove_dir_all(temp);
    }
}
