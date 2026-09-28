//! Validated identifiers.
//!
//! Identifiers end up in file paths, config keys, environment variables and command
//! lines in later phases, so they are restricted to a small, unambiguous character
//! set when parsed instead of being escaped at every use site.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid {kind} {value:?}: {rule}")]
pub struct InvalidName {
    pub kind: &'static str,
    pub value: String,
    pub rule: &'static str,
}

/// Identifies an agent, model provider or MCP server definition, e.g. `claude-code`.
///
/// 1–64 characters of `a-z`, `0-9` and `-`, starting with a letter and not ending
/// with `-`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(export)]
pub struct IntegrationId(String);

impl IntegrationId {
    const MAX_LEN: usize = 64;
    const RULE: &'static str = "must be 1-64 characters of a-z, 0-9 and '-', \
                                starting with a letter and not ending with '-'";

    pub fn new(value: impl Into<String>) -> Result<Self, InvalidName> {
        let value = value.into();
        let bytes = value.as_bytes();
        let valid = !bytes.is_empty()
            && bytes.len() <= Self::MAX_LEN
            && bytes[0].is_ascii_lowercase()
            && bytes[bytes.len() - 1] != b'-'
            && bytes
                .iter()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-');
        if valid {
            Ok(Self(value))
        } else {
            Err(InvalidName {
                kind: "integration id",
                value,
                rule: Self::RULE,
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Names a secret held by the native secret store, e.g. `ANTHROPIC_API_KEY`.
///
/// Definitions refer to secrets only by name; the value is resolved by the native
/// layer when a process is launched and is never part of a definition, a log line or
/// an IPC payload. Uses the same rules as a POSIX environment variable name because
/// secrets are usually delivered to child processes as environment variables.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(try_from = "String", into = "String")]
#[ts(export)]
pub struct SecretName(String);

impl SecretName {
    pub fn new(value: impl Into<String>) -> Result<Self, InvalidName> {
        let value = value.into();
        if is_env_var_name(&value) {
            Ok(Self(value))
        } else {
            Err(InvalidName {
                kind: "secret name",
                value,
                rule: ENV_VAR_NAME_RULE,
            })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

pub(crate) const ENV_VAR_NAME_RULE: &str = "must be 1-128 characters of A-Z, a-z, 0-9 and '_', \
                                            not starting with a digit";

/// `[A-Za-z_][A-Za-z0-9_]{0,127}`
pub(crate) fn is_env_var_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && !bytes[0].is_ascii_digit()
        && bytes
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'_')
}

macro_rules! string_newtype_impls {
    ($name:ident) => {
        impl TryFrom<String> for $name {
            type Error = InvalidName;
            fn try_from(value: String) -> Result<Self, Self::Error> {
                Self::new(value)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}

string_newtype_impls!(IntegrationId);
string_newtype_impls!(SecretName);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integration_id_accepts_kebab_case() {
        for ok in [
            "claude-code",
            "opencode",
            "qwen3-coder",
            "a",
            "mcp-github-2",
        ] {
            assert!(IntegrationId::new(ok).is_ok(), "{ok} should be valid");
        }
    }

    #[test]
    fn integration_id_rejects_path_and_shell_hazards() {
        let too_long = "a".repeat(65);
        for bad in [
            "",
            "Claude",
            "1password",
            "-lead",
            "trail-",
            "../etc",
            "a/b",
            "a b",
            "a;rm",
            "ünicode",
            too_long.as_str(),
        ] {
            assert!(
                IntegrationId::new(bad).is_err(),
                "{bad:?} should be invalid"
            );
        }
    }

    #[test]
    fn secret_name_follows_env_var_rules() {
        assert!(SecretName::new("ANTHROPIC_API_KEY").is_ok());
        assert!(SecretName::new("_private").is_ok());
        for bad in ["", "1KEY", "MY-KEY", "KEY=1", "KEY ", "$HOME"] {
            assert!(SecretName::new(bad).is_err(), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn deserialization_validates() {
        assert!(serde_json::from_str::<IntegrationId>("\"../escape\"").is_err());
        assert!(serde_json::from_str::<SecretName>("\"NOT-VALID\"").is_err());
    }
}
