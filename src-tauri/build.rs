/// Every app command must be listed here. Tauri then generates `allow-*`/`deny-*`
/// permissions for it, and the command is rejected unless a capability in
/// `capabilities/` grants it to the calling window.
const COMMANDS: &[&str] = &["get_app_info"];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
