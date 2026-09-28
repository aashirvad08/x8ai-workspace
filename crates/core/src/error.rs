//! The error every native command returns to the frontend.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Serialized as `{ "code": "...", "message": "..." }`.
///
/// Commands return `Result<T, CommandError>`. `message` is shown to humans and must
/// never contain secret values or full environment dumps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct CommandError {
    pub code: ErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum ErrorCode {
    /// The request was malformed or failed validation.
    InvalidInput,
    /// The referenced session, workspace, file or integration does not exist.
    NotFound,
    /// The target of a create or rename already exists.
    AlreadyExists,
    /// The file changed or disappeared on disk since it was read. Nothing was
    /// written; the caller decides whether to overwrite or reload.
    Conflict,
    /// The action was refused by a policy or by the user.
    PermissionDenied,
    /// Anything else. Details are logged on the native side.
    Internal,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}: {}", self.code, self.message)
    }
}

impl std::error::Error for CommandError {}
