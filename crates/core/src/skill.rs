//! Skills (Phase 8, docs/catalog.md): instructions for an agent, attached to an
//! agent session. A skill is text, never a program: it runs nothing, holds no
//! secret, and cannot change a provider, an MCP server, an agent's configuration,
//! trust or approvals. An agent's adapter passes it to the agent for one session.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::definition::{DefinitionError, check_name};
use crate::id::IntegrationId;
use crate::mcp::{McpAgentSupport, looks_like_credential};

/// Longest instructions a skill may have.
pub const MAX_INSTRUCTIONS_BYTES: usize = 16 * 1024;
/// Most tools a skill may suggest.
pub const MAX_TOOLS: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct Skill {
    /// Stable: made from the name when a user skill is added.
    pub id: IntegrationId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// The skill's own version: a built-in skill's catalog version, or how many
    /// times a user skill has been saved.
    pub version: u32,
    /// What the agent is told.
    pub instructions: String,
    /// Tools the skill suggests (for example `Bash`, `Read`). Shown to the user;
    /// never granted or passed to the agent as permissions.
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub source: SkillSource,
    pub scope: SkillScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum SkillSource {
    /// Shipped with the app, reviewed with it.
    Builtin,
    /// Written by the user in the app.
    User,
}

/// Which sessions a skill is attached to, as for MCP servers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub enum SkillScope {
    Global,
    Workspace { root: String },
    Session,
}

/// A user skill as the webview adds or changes it. The id, the version and a
/// workspace scope's folder are decided natively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct SkillInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub instructions: String,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    pub scope: crate::mcp::McpScopeKind,
}

/// What a session recorded about a skill: exactly which one, as it was then.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct SkillRef {
    pub id: IntegrationId,
    pub version: u32,
    /// A digest of the name and instructions, to tell when a skill changed.
    pub fingerprint: String,
}

impl SkillInput {
    pub fn validate(&self) -> Result<(), DefinitionError> {
        check_name(&self.name)?;
        check_text("description", &self.description, 500, false)?;
        check_text(
            "instructions",
            &self.instructions,
            MAX_INSTRUCTIONS_BYTES,
            true,
        )?;
        check_tools(&self.allowed_tools)
    }
}

impl Skill {
    pub fn validate(&self) -> Result<(), DefinitionError> {
        check_name(&self.name)?;
        check_text("description", &self.description, 500, false)?;
        check_text(
            "instructions",
            &self.instructions,
            MAX_INSTRUCTIONS_BYTES,
            true,
        )?;
        check_tools(&self.allowed_tools)?;
        if let SkillScope::Workspace { root } = &self.scope
            && !root.starts_with('/')
        {
            return Err(DefinitionError::new(
                "scope.root",
                "must be an absolute path",
            ));
        }
        Ok(())
    }

    /// FNV-1a over the name and instructions: detects a change, nothing more.
    pub fn fingerprint(&self) -> String {
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for byte in self
            .name
            .bytes()
            .chain([0])
            .chain(self.instructions.bytes())
        {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0100_0000_01b3);
        }
        format!("{hash:016x}")
    }

    pub fn reference(&self) -> SkillRef {
        SkillRef {
            id: self.id.clone(),
            version: self.version,
            fingerprint: self.fingerprint(),
        }
    }
}

fn check_text(field: &str, text: &str, max: usize, required: bool) -> Result<(), DefinitionError> {
    if required && text.trim().is_empty() {
        return Err(DefinitionError::new(field, "must not be empty"));
    }
    if text.len() > max {
        return Err(DefinitionError::new(
            field,
            format!("must be at most {max} bytes"),
        ));
    }
    if text
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(DefinitionError::new(
            field,
            "must not contain control characters",
        ));
    }
    // A skill is plain text anyone may read: never a place for a key.
    if text
        .split(|c: char| c.is_whitespace() || "\"'`()<>[]{},;".contains(c))
        .any(looks_like_credential)
    {
        return Err(DefinitionError::new(
            field,
            "looks like it contains a credential; skills must not hold secrets",
        ));
    }
    Ok(())
}

fn check_tools(tools: &[String]) -> Result<(), DefinitionError> {
    if tools.len() > MAX_TOOLS {
        return Err(DefinitionError::new(
            "allowedTools",
            format!("at most {MAX_TOOLS}"),
        ));
    }
    for (i, tool) in tools.iter().enumerate() {
        let ok = !tool.is_empty()
            && tool.len() <= 100
            && tool
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-:.*() ".contains(c));
        if !ok {
            return Err(DefinitionError::new(
                format!("allowedTools[{i}]"),
                "must be a tool name: letters, digits and _ - : . * ( )",
            ));
        }
    }
    Ok(())
}

// IPC contracts.

/// A skill, and which agents can take it. Returned by `skill_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SkillStatus {
    pub skill: Skill,
    pub agents: Vec<McpAgentSupport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SkillList {
    pub skills: Vec<SkillStatus>,
}

/// A skill attached to an agent session, and whether it can run as recorded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionSkill {
    pub id: IntegrationId,
    pub name: String,
    pub version: u32,
    pub state: SessionSkillState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
#[ts(export)]
pub enum SessionSkillState {
    /// As recorded: the session runs with it.
    Attached,
    /// Changed since the session was created: the session does not run until a
    /// new session is started (never silently upgraded).
    Changed { current_version: u32 },
    /// Removed from the app: the session does not run.
    Removed,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(instructions: &str) -> SkillInput {
        SkillInput {
            name: "Python debugging".into(),
            description: "Reproduce first".into(),
            instructions: instructions.into(),
            allowed_tools: vec!["Bash".into(), "Read".into()],
            scope: crate::mcp::McpScopeKind::Session,
        }
    }

    #[test]
    fn a_skill_is_text_without_secrets() {
        assert!(
            input("Reproduce the failure first.\n\n- Read the traceback.\n\tThen fix.")
                .validate()
                .is_ok()
        );
        assert!(input("").validate().is_err());
        assert!(input("bell\u{7}").validate().is_err());
        assert!(
            input(&"x".repeat(MAX_INSTRUCTIONS_BYTES + 1))
                .validate()
                .is_err()
        );
        let with_key = format!("Use the key ghp_{} to push.", "a".repeat(36));
        let error = input(&with_key).validate().unwrap_err();
        assert!(error.to_string().contains("credential") && !error.to_string().contains("ghp_"));
        assert!(
            input(&format!("export OPENAI_API_KEY=\"sk-{}\"", "b".repeat(40)))
                .validate()
                .is_err()
        );
        let mut tools = input("ok");
        tools.allowed_tools = vec!["Bash; rm -rf ~".into()];
        assert!(tools.validate().is_err());
    }

    #[test]
    fn the_fingerprint_changes_with_the_instructions_only_where_it_matters() {
        let skill = Skill {
            id: IntegrationId::new("python-debugging").unwrap(),
            name: "Python debugging".into(),
            description: String::new(),
            version: 1,
            instructions: "Reproduce first.".into(),
            allowed_tools: Vec::new(),
            source: SkillSource::Builtin,
            scope: SkillScope::Session,
        };
        let same = Skill {
            description: "another description".into(),
            ..skill.clone()
        };
        assert_eq!(skill.fingerprint(), same.fingerprint());
        let changed = Skill {
            instructions: "Guess first.".into(),
            ..skill.clone()
        };
        assert_ne!(skill.fingerprint(), changed.fingerprint());
        assert_eq!(skill.reference().version, 1);
    }
}
