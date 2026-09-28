//! How an external program is started.
//!
//! Everything the app runs locally on the user's behalf — coding agents, stdio MCP
//! servers and, later, language servers — is described by a [`LaunchSpec`]. The spec
//! is what the user reviews and what the native layer executes, and nothing else, so
//! it is fully explicit: one program, its arguments, and the environment variables
//! added for it. It is never passed through a shell.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::definition::DefinitionError;
use crate::id::{ENV_VAR_NAME_RULE, SecretName, is_env_var_name};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct LaunchSpec {
    /// Executable name or absolute path. Names are resolved against the user's
    /// login-shell `PATH` by the native layer (Phase 4), not the GUI app's `PATH`.
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// Variables set for this process on top of the environment the native layer
    /// provides. Secret values are referenced by name only.
    #[serde(default)]
    pub env: Vec<EnvVar>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct EnvVar {
    pub name: String,
    pub value: EnvValue,
}

/// Serialized as `{ "literal": "..." }` or `{ "secret": "NAME" }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum EnvValue {
    /// A plain, non-sensitive value stored in the definition.
    Literal(String),
    /// Resolved from the native secret store at launch time, for this process only.
    Secret(SecretName),
}

/// Something that must exist on the machine before an integration can run.
///
/// Requirements are checked and reported; they are never installed silently.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub enum Requirement {
    /// An executable that must be resolvable, e.g. `node`, `docker` or `claude`.
    Executable { program: String },
}

impl LaunchSpec {
    pub fn validate(&self, field: &str) -> Result<(), DefinitionError> {
        check_command_word(&format!("{field}.program"), &self.program)?;
        for (i, arg) in self.args.iter().enumerate() {
            if arg.contains('\0') {
                return Err(DefinitionError::new(
                    format!("{field}.args[{i}]"),
                    "must not contain a NUL byte",
                ));
            }
        }
        let mut seen = std::collections::BTreeSet::new();
        for (i, var) in self.env.iter().enumerate() {
            let at = format!("{field}.env[{i}]");
            if !is_env_var_name(&var.name) {
                return Err(DefinitionError::new(
                    format!("{at}.name"),
                    ENV_VAR_NAME_RULE,
                ));
            }
            if !seen.insert(var.name.as_str()) {
                return Err(DefinitionError::new(
                    format!("{at}.name"),
                    format!("{} is set more than once", var.name),
                ));
            }
            if let EnvValue::Literal(value) = &var.value
                && value.contains('\0')
            {
                return Err(DefinitionError::new(
                    format!("{at}.value"),
                    "must not contain a NUL byte",
                ));
            }
        }
        Ok(())
    }
}

impl Requirement {
    pub fn validate(&self, field: &str) -> Result<(), DefinitionError> {
        match self {
            Self::Executable { program } => {
                check_command_word(&format!("{field}.program"), program)
            }
        }
    }
}

fn check_command_word(field: &str, value: &str) -> Result<(), DefinitionError> {
    if value.trim().is_empty() {
        return Err(DefinitionError::new(field, "must not be empty"));
    }
    if value.contains('\0') {
        return Err(DefinitionError::new(field, "must not contain a NUL byte"));
    }
    if value.trim() != value {
        return Err(DefinitionError::new(
            field,
            "must not have leading or trailing whitespace",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(env: Vec<EnvVar>) -> LaunchSpec {
        LaunchSpec {
            program: "npx".into(),
            args: vec!["@playwright/mcp@0.0.40".into()],
            env,
        }
    }

    #[test]
    fn env_values_serialize_as_tagged_objects() {
        let var = EnvVar {
            name: "GITHUB_PERSONAL_ACCESS_TOKEN".into(),
            value: EnvValue::Secret(SecretName::new("GITHUB_TOKEN").unwrap()),
        };
        let json = serde_json::to_value(&var).unwrap();
        assert_eq!(
            json,
            serde_json::json!({ "name": "GITHUB_PERSONAL_ACCESS_TOKEN", "value": { "secret": "GITHUB_TOKEN" } })
        );
    }

    #[test]
    fn rejects_empty_program() {
        let mut s = spec(vec![]);
        s.program = "  ".into();
        let err = s.validate("launch").unwrap_err();
        assert_eq!(err.field, "launch.program");
    }

    #[test]
    fn rejects_invalid_and_duplicate_env_names() {
        let literal = |name: &str| EnvVar {
            name: name.into(),
            value: EnvValue::Literal("1".into()),
        };
        assert!(spec(vec![literal("DEBUG")]).validate("launch").is_ok());
        assert!(spec(vec![literal("NOT-VALID")]).validate("launch").is_err());
        let err = spec(vec![literal("DEBUG"), literal("DEBUG")])
            .validate("launch")
            .unwrap_err();
        assert_eq!(err.field, "launch.env[1].name");
    }

    #[test]
    fn rejects_nul_bytes_in_args() {
        let mut s = spec(vec![]);
        s.args.push("a\0b".into());
        assert_eq!(s.validate("launch").unwrap_err().field, "launch.args[1]");
    }
}
