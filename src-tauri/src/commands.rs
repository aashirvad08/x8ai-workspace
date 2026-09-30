//! IPC commands exposed to the webview.
//!
//! Adding a command takes four steps (see `docs/architecture.md`, "Adding a native
//! command"): define it here, register it in `lib.rs`, list it in `build.rs`, and
//! grant it to a window in `capabilities/`. A command without a grant is rejected by
//! Tauri before it runs.
//!
//! Every argument is untrusted input from the webview and must be validated here.

use tauri::AppHandle;
use x8ai_core::app::AppInfo;

#[tauri::command]
pub fn get_app_info(app: AppHandle) -> AppInfo {
    let package = app.package_info();
    // The account's full name, for the welcome screen; read from the user
    // database, never from the environment.
    let account = nix::unistd::User::from_uid(nix::unistd::getuid())
        .ok()
        .flatten();
    let gecos = account.as_ref().and_then(|a| a.gecos.to_str().ok());
    AppInfo::for_current_platform(package.name.clone(), package.version.to_string())
        .with_user_name(gecos)
}
