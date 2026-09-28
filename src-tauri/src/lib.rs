//! Native host for x8ai Workspace.
//!
//! This crate is the only one that knows about Tauri. It owns the window and menu,
//! registers the IPC commands the webview may call, and holds native state:
//! terminal sessions, the open workspace, and the quit guard. Logic that does not
//! need Tauri belongs in `crates/`.

mod app;
mod commands;
mod menu;
mod terminal;
mod workspace;

use tauri::webview::PageLoadEvent;
use tauri::{Manager, RunEvent, WindowEvent};

use app::AppState;
use terminal::Terminals;
use workspace::Workspaces;

pub fn run() {
    tauri::Builder::default()
        // Only the Rust API (the native folder picker) is used. None of the
        // plugin's webview commands are granted in `capabilities/`.
        .plugin(tauri_plugin_dialog::init())
        .manage(Terminals::default())
        .manage(Workspaces::default())
        .manage(AppState::default())
        .menu(menu::build)
        .on_menu_event(|app, event| {
            if event.id() == menu::QUIT {
                app::quit_requested(app);
            }
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let state = window.state::<AppState>();
                if state.must_ask() {
                    api.prevent_close();
                    state.ask();
                }
            }
        })
        .on_page_load(|webview, payload| {
            // Native state belongs to the page that created it. A reload starts a
            // new page, so release the old page's sessions, workspace and quit
            // guard instead of leaking them.
            if payload.event() == PageLoadEvent::Started {
                webview.state::<Terminals>().close_all();
                webview.state::<Workspaces>().close();
                webview.state::<AppState>().reset();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            app::app_subscribe,
            app::app_set_unsaved_changes,
            app::app_quit,
            terminal::terminal_create,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_ack,
            terminal::terminal_close,
            workspace::workspace_open,
            workspace::workspace_list_dir,
            workspace::workspace_read_file,
            workspace::workspace_file_version,
            workspace::workspace_write_file,
            workspace::workspace_create_file,
            workspace::workspace_create_dir,
            workspace::workspace_rename,
            workspace::workspace_delete,
            workspace::workspace_list_files,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start the x8ai Workspace host")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<Terminals>().shutdown();
            }
        });
}
