use std::io;

/// Why a workspace operation failed. Every variant names the workspace path, so
/// messages are useful when shown to the user.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{path:?} {reason}")]
    InvalidPath { path: String, reason: &'static str },
    #[error("{0:?} does not exist")]
    NotFound(String),
    #[error("{0:?} already exists")]
    AlreadyExists(String),
    /// Includes attempts to reach outside the workspace, e.g. through a symlink.
    #[error("{path:?}: permission denied ({detail})")]
    PermissionDenied { path: String, detail: String },
    #[error("{path:?} {}", .reason.describe())]
    Conflict {
        path: String,
        reason: ConflictReason,
    },
    /// Binary (contains a NUL byte) or not UTF-8.
    #[error("{0:?} is a binary file, or text that is not UTF-8, so it is not opened")]
    NotText(String),
    #[error("{path:?} is {size} bytes; files over {max} bytes are not opened")]
    TooLarge { path: String, size: u64, max: u64 },
    #[error("{path:?}: {detail}")]
    Io { path: String, detail: String },
    #[error("invalid search: {0}")]
    InvalidQuery(String),
    /// A remembered workspace path now resolves to a different folder.
    #[error("{path:?} now leads to {now:?}, a different folder")]
    Moved { path: String, now: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictReason {
    Modified,
    Deleted,
}

impl ConflictReason {
    fn describe(self) -> &'static str {
        match self {
            Self::Modified => "changed on disk since it was opened",
            Self::Deleted => "was deleted on disk since it was opened",
        }
    }
}

impl Error {
    pub(crate) fn invalid(path: &str, reason: &'static str) -> Self {
        Self::InvalidPath {
            path: path.to_owned(),
            reason,
        }
    }

    /// Maps an I/O error for `path` onto the variants callers act on.
    pub(crate) fn io(path: &str, error: io::Error) -> Self {
        let path = path.to_owned();
        match error.kind() {
            io::ErrorKind::NotFound => Self::NotFound(path),
            io::ErrorKind::AlreadyExists => Self::AlreadyExists(path),
            io::ErrorKind::PermissionDenied => Self::PermissionDenied {
                path,
                detail: error.to_string(),
            },
            _ => Self::Io {
                path,
                detail: error.to_string(),
            },
        }
    }
}
