/// Every app command must be listed here. Tauri then generates `allow-*`/`deny-*`
/// permissions for it, and the command is rejected unless a capability in
/// `capabilities/` grants it to the calling window.
const COMMANDS: &[&str] = &[
    "get_app_info",
    "app_subscribe",
    "app_set_unsaved_changes",
    "app_quit",
    "terminal_create",
    "terminal_write",
    "terminal_resize",
    "terminal_ack",
    "terminal_close",
    "workspace_open",
    "workspace_list_dir",
    "workspace_read_file",
    "workspace_file_version",
    "workspace_write_file",
    "workspace_create_file",
    "workspace_create_dir",
    "workspace_rename",
    "workspace_delete",
    "workspace_list_files",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
