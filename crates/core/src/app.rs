//! Identity of the running application.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Returned by the `get_app_info` command. Proves the webview can reach the native
/// host and tells the UI what it is running on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
    /// Operating system as reported by Rust: `macos`, `linux`, ...
    pub os: String,
    /// CPU architecture as reported by Rust: `aarch64`, `x86_64`, ...
    pub arch: String,
}

impl AppInfo {
    /// Describes the current process. `name` and `version` come from the host's
    /// package metadata; the platform fields are fixed at compile time.
    pub fn for_current_platform(name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: version.into(),
            os: std::env::consts::OS.to_owned(),
            arch: std::env::consts::ARCH.to_owned(),
        }
    }
}

/// App-level events delivered on the channel given to `app_subscribe`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum AppEvent {
    /// The user asked to quit or close the window while the frontend reported
    /// unsaved changes. Nothing closes until the frontend calls `app_quit`.
    QuitRequested,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_the_compile_time_platform() {
        let info = AppInfo::for_current_platform("x8ai", "1.2.3");
        assert_eq!(info.os, std::env::consts::OS);
        assert_eq!(info.arch, std::env::consts::ARCH);
        assert_eq!(info.version, "1.2.3");
    }
}
