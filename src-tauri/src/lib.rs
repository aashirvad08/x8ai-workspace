//! Native host for x8ai Workspace.
//!
//! This crate is the only one that knows about Tauri. It owns the window, registers
//! the IPC commands the webview may call and, from Phase 1, holds native state such
//! as terminal sessions. Logic that does not need Tauri belongs in `crates/`.

mod commands;

pub fn run() {
    // TODO(phase-1): `.manage()` the terminal session registry, kill every session's
    // process group on exit, and register the terminal_* commands (ADR 0006).
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![commands::get_app_info])
        .run(tauri::generate_context!())
        .expect("failed to start the x8ai Workspace host");
}
