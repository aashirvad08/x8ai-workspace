//! Built-in skills, and the user's in `skills.json`.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use x8ai_core::definition::DefinitionError;
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::McpScopeKind;
use x8ai_core::skill::{Skill, SkillInput, SkillScope, SkillSource};

/// Most user skills kept.
pub const MAX_SKILLS: usize = 200;
const VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 8 << 20;

/// The skills that ship with the app, in the order shown.
pub fn builtin() -> Vec<Skill> {
    serde_json::from_str(include_str!("builtin.json")).expect("builtin.json is checked by tests")
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(#[from] DefinitionError),
    #[error("no skill {0:?}")]
    NotFound(String),
    #[error("{0} is built in; it cannot be changed or removed")]
    Builtin(String),
    #[error("{0} is attached to every session it applies to; it cannot be chosen per session")]
    NotSessionScoped(String),
    #[error("open a folder first: a workspace skill belongs to the open folder")]
    NoWorkspace,
    #[error("at most {MAX_SKILLS} skills can be added")]
    Full,
    #[error("{path}: {detail}")]
    Io { path: String, detail: String },
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    skills: Vec<serde_json::Value>,
}

#[derive(Debug)]
pub struct SkillRegistry {
    file: PathBuf,
    builtin: Vec<Skill>,
    user: Vec<Skill>,
}

impl SkillRegistry {
    /// Loads the user's skills; a missing file is none. A damaged file is set
    /// aside; an entry that does not validate, claims to be built in, or reuses a
    /// built-in id is dropped. Returns warnings for each.
    pub fn load(file: PathBuf) -> (Self, Vec<String>) {
        let builtin = builtin();
        let mut warnings = Vec::new();
        let mut user: Vec<Skill> = Vec::new();
        let parsed = match fs::metadata(&file) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
            Ok(m) if m.len() > MAX_FILE_BYTES => Err(format!("{} bytes is too large", m.len())),
            Ok(_) => fs::read(&file).map_err(|e| e.to_string()).and_then(|b| {
                serde_json::from_slice::<File>(&b)
                    .map(Some)
                    .map_err(|e| e.to_string())
            }),
        };
        match parsed {
            Ok(Some(parsed)) if parsed.version == VERSION => {
                for value in parsed.skills {
                    let skill = serde_json::from_value::<Skill>(value)
                        .map_err(|e| e.to_string())
                        .and_then(|s| s.validate().map(|()| s).map_err(|e| e.to_string()));
                    match skill {
                        Ok(s) if s.source != SkillSource::User => {
                            warnings.push(format!(
                                "skill {} in {} claims to be built in; ignored",
                                s.id,
                                file.display()
                            ));
                        }
                        Ok(s) if builtin.iter().chain(&user).any(|b| b.id == s.id) => {
                            warnings.push(format!(
                                "skill {} is listed twice; the second was ignored",
                                s.id
                            ));
                        }
                        Ok(s) => user.push(s),
                        Err(reason) => warnings.push(format!(
                            "a skill in {} was ignored: {reason}",
                            file.display()
                        )),
                    }
                }
            }
            Ok(Some(parsed)) => warnings.push(format!(
                "{} has version {}, which this app does not read; no user skills loaded",
                file.display(),
                parsed.version
            )),
            Ok(None) => {}
            Err(reason) => {
                let aside = file.with_extension("json.corrupt");
                let _ = fs::rename(&file, &aside);
                warnings.push(format!(
                    "{} was damaged ({reason}); starting without user skills. The old file is at {}",
                    file.display(),
                    aside.display()
                ));
            }
        }
        user.truncate(MAX_SKILLS);
        (
            Self {
                file,
                builtin,
                user,
            },
            warnings,
        )
    }

    /// Built-in skills, then the user's.
    pub fn skills(&self) -> Vec<Skill> {
        self.builtin.iter().chain(&self.user).cloned().collect()
    }

    pub fn get(&self, id: &str) -> Option<&Skill> {
        self.builtin
            .iter()
            .chain(&self.user)
            .find(|s| s.id.as_str() == id)
    }

    pub fn add(&mut self, input: &SkillInput, workspace: Option<&Path>) -> Result<Skill, Error> {
        input.validate()?;
        if self.user.len() >= MAX_SKILLS {
            return Err(Error::Full);
        }
        let skill = Skill {
            id: self.new_id(&input.name),
            name: input.name.trim().to_owned(),
            description: input.description.trim().to_owned(),
            version: 1,
            instructions: input.instructions.trim().to_owned(),
            allowed_tools: input.allowed_tools.clone(),
            source: SkillSource::User,
            scope: scope(input.scope, workspace, None)?,
        };
        skill.validate()?;
        self.user.push(skill.clone());
        self.save()?;
        Ok(skill)
    }

    /// Changes a user skill; its version goes up when what the agent is told
    /// changes, so sessions that recorded the old one notice.
    pub fn update(
        &mut self,
        id: &str,
        input: &SkillInput,
        workspace: Option<&Path>,
    ) -> Result<Skill, Error> {
        input.validate()?;
        let index = self.user_index(id)?;
        let old = self.user[index].clone();
        let mut skill = Skill {
            id: old.id.clone(),
            name: input.name.trim().to_owned(),
            description: input.description.trim().to_owned(),
            version: old.version,
            instructions: input.instructions.trim().to_owned(),
            allowed_tools: input.allowed_tools.clone(),
            source: SkillSource::User,
            scope: scope(input.scope, workspace, Some(&old.scope))?,
        };
        if skill.fingerprint() != old.fingerprint() {
            skill.version = old.version.saturating_add(1);
        }
        skill.validate()?;
        self.user[index] = skill.clone();
        self.save()?;
        Ok(skill)
    }

    pub fn remove(&mut self, id: &str) -> Result<Skill, Error> {
        let index = self.user_index(id)?;
        let removed = self.user.remove(index);
        self.save()?;
        Ok(removed)
    }

    fn user_index(&self, id: &str) -> Result<usize, Error> {
        if let Some(builtin) = self.builtin.iter().find(|s| s.id.as_str() == id) {
            return Err(Error::Builtin(builtin.name.clone()));
        }
        self.user
            .iter()
            .position(|s| s.id.as_str() == id)
            .ok_or_else(|| Error::NotFound(id.to_owned()))
    }

    fn new_id(&self, name: &str) -> IntegrationId {
        let mut slug = String::new();
        for c in name.trim().chars().flat_map(char::to_lowercase) {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                slug.push(c);
            } else if !slug.ends_with('-') && !slug.is_empty() {
                slug.push('-');
            }
        }
        let slug: String = slug
            .trim_start_matches(|c: char| !c.is_ascii_lowercase())
            .chars()
            .take(48)
            .collect();
        let slug = slug.trim_end_matches('-');
        let base = if slug.is_empty() { "skill" } else { slug };
        (1..)
            .map(|n| {
                if n == 1 {
                    base.to_owned()
                } else {
                    format!("{base}-{n}")
                }
            })
            .find(|candidate| self.get(candidate).is_none())
            .and_then(|id| IntegrationId::new(id).ok())
            .expect("a free id")
    }

    fn save(&self) -> Result<(), Error> {
        let io = |e: std::io::Error| Error::Io {
            path: self.file.display().to_string(),
            detail: e.to_string(),
        };
        let json = serde_json::to_vec_pretty(&File {
            version: VERSION,
            skills: self
                .user
                .iter()
                .map(|s| serde_json::to_value(s).expect("serializable"))
                .collect(),
        })
        .expect("serializable");
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir).map_err(io)?;
            fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(io)?;
        }
        let temp = self.file.with_extension("json.tmp");
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)
            .map_err(io)?;
        out.write_all(&json)
            .and_then(|()| out.sync_all())
            .map_err(io)?;
        fs::rename(&temp, &self.file).map_err(io)
    }
}

fn scope(
    kind: McpScopeKind,
    workspace: Option<&Path>,
    previous: Option<&SkillScope>,
) -> Result<SkillScope, Error> {
    Ok(match kind {
        McpScopeKind::Global => SkillScope::Global,
        McpScopeKind::Session => SkillScope::Session,
        McpScopeKind::Workspace => match (workspace, previous) {
            (Some(root), _) => SkillScope::Workspace {
                root: root.display().to_string(),
            },
            (None, Some(previous @ SkillScope::Workspace { .. })) => previous.clone(),
            (None, _) => return Err(Error::NoWorkspace),
        },
    })
}
