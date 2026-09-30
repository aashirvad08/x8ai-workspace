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
    /// The full name of the user's account, if it has one, for the welcome
    /// screen. Nothing else is read about the user.
    pub user_name: Option<String>,
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
            user_name: None,
        }
    }

    /// The account's full name, from the first field of its GECOS entry (where
    /// macOS and Linux keep it), trimmed; `None` if it is empty.
    pub fn with_user_name(mut self, gecos: Option<&str>) -> Self {
        self.user_name = gecos
            .and_then(|g| g.split(',').next())
            .map(str::trim)
            .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
            .map(str::to_owned);
        self
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
        assert_eq!(info.user_name, None);
    }

    #[test]
    fn the_user_name_is_the_first_gecos_field() {
        let named = |gecos| AppInfo::for_current_platform("x8ai", "1").with_user_name(gecos);
        assert_eq!(
            named(Some("Ada Lovelace")).user_name.as_deref(),
            Some("Ada Lovelace")
        );
        assert_eq!(
            named(Some(" Ada Lovelace ,Room 1,,")).user_name.as_deref(),
            Some("Ada Lovelace")
        );
        assert_eq!(named(Some("")).user_name, None);
        assert_eq!(named(Some(",,,")).user_name, None);
        assert_eq!(named(Some("a\u{7}b")).user_name, None);
        assert_eq!(named(None).user_name, None);
    }
}
