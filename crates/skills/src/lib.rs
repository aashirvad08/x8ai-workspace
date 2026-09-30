//! Skills (docs/catalog.md): instructions an agent gets for one session.
//!
//! A skill is text: a name, a description, its instructions, the tools it
//! suggests, where it comes from and which sessions it is attached to. It runs
//! nothing, holds no secret (validation refuses credential-like text), and has
//! no way to change a provider, an MCP server, an agent's configuration, trust or
//! approvals. Built-in skills ship with the app; user skills are in
//! `skills.json` (0600). An agent's adapter (`x8ai-agents`) passes the skills of
//! a session to the agent, for that session only.
//!
//! A session records each skill's id, version and fingerprint. A later run uses
//! exactly those, or refuses to run if one was removed or changed: a skill is
//! never silently upgraded or substituted. No Tauri dependency.

#![forbid(unsafe_code)]

mod registry;

pub use registry::{Error, SkillRegistry, builtin};

use std::path::Path;

use x8ai_core::id::IntegrationId;
use x8ai_core::skill::{Skill, SkillRef, SkillScope};

/// The skills a new session in `root` gets: every global skill, every skill of
/// this workspace, and the session skills in `chosen`.
pub fn attach<'s>(
    skills: &'s [Skill],
    root: &Path,
    chosen: &[IntegrationId],
) -> Result<Vec<&'s Skill>, Error> {
    for id in chosen {
        let skill = skills
            .iter()
            .find(|s| s.id == *id)
            .ok_or_else(|| Error::NotFound(id.to_string()))?;
        if skill.scope != SkillScope::Session {
            return Err(Error::NotSessionScoped(skill.name.clone()));
        }
    }
    Ok(skills
        .iter()
        .filter(|s| match &s.scope {
            SkillScope::Global => true,
            SkillScope::Workspace { root: r } => Path::new(r) == root,
            SkillScope::Session => chosen.contains(&s.id),
        })
        .collect())
}

/// Why a session cannot run with the skills it recorded.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Mismatch {
    #[error("the skill {0} was removed; start a new session without it")]
    Removed(String),
    #[error(
        "the skill {name} changed since this session was created (version {was} → {now}); \
         start a new session to use it"
    )]
    Changed { name: String, was: u32, now: u32 },
}

/// The skills exactly as a session recorded them, or why not.
pub fn resolve<'s>(skills: &'s [Skill], recorded: &[SkillRef]) -> Result<Vec<&'s Skill>, Mismatch> {
    recorded
        .iter()
        .map(|r| {
            let skill = skills
                .iter()
                .find(|s| s.id == r.id)
                .ok_or_else(|| Mismatch::Removed(r.id.to_string()))?;
            if skill.fingerprint() != r.fingerprint || skill.version != r.version {
                return Err(Mismatch::Changed {
                    name: skill.name.clone(),
                    was: r.version,
                    now: skill.version,
                });
            }
            Ok(skill)
        })
        .collect()
}
