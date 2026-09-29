//! Native host for x8ai Workspace.
//!
//! This crate is the only one that knows about Tauri. It owns the window and menu,
//! registers the IPC commands the webview may call, and holds native state:
//! terminal sessions, agents, model providers, MCP servers, the open workspace,
//! and the quit guard. Logic that does not
//! need Tauri belongs in `crates/`.

mod agents;
mod app;
mod commands;
#[cfg(target_os = "macos")]
mod macos;
mod mcp;
mod menu;
mod providers;
mod terminal;
mod workspace;

use tauri::webview::PageLoadEvent;
use tauri::{Manager, RunEvent, WindowEvent};

use agents::Agents;
use app::AppState;
use mcp::Mcp;
use providers::Providers;
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
        .manage(Agents::default())
        .manage(Providers::default())
        .manage(Mcp::default())
        .setup(|app| {
            let workspaces = app.state::<Workspaces>();
            match app.path().app_data_dir() {
                Ok(data_dir) => {
                    workspaces.load_stores(&data_dir);
                    app.state::<Providers>()
                        .load_settings(&data_dir, &workspaces);
                    // Loads the registry and approvals; starts no server.
                    app.state::<Mcp>().load(&data_dir, &workspaces);
                }
                // The app still works; it just cannot remember folders or trust.
                Err(e) => workspaces.warn(format!("Recent folders and trust are unavailable: {e}")),
            }
            #[cfg(target_os = "macos")]
            if let Err(reason) = macos::intercept_termination(app.handle()) {
                // Quit from the Dock or logout then skips the unsaved-changes
                // question; everything else still works.
                eprintln!("x8ai: cannot intercept system quit: {reason}");
            }
            Ok(())
        })
        .menu(menu::build)
        .on_menu_event(|app, event| {
            if event.id() == menu::QUIT {
                app::quit_requested(app);
            }
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && app::must_ask(window.app_handle())
            {
                api.prevent_close();
                window.state::<AppState>().ask();
            }
        })
        .on_page_load(|webview, payload| {
            // Native state belongs to the page that created it. A reload starts a
            // new page, so release the old page's sessions, workspace and quit
            // guard instead of leaking them.
            if payload.event() == PageLoadEvent::Started {
                webview.state::<Terminals>().close_all();
                webview.state::<Agents>().forget_all();
                let app = webview.app_handle().clone();
                std::thread::spawn(move || app.state::<Mcp>().stop_all());
                webview.state::<Workspaces>().close();
                webview.state::<AppState>().reset();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            app::app_subscribe,
            app::app_set_unsaved_changes,
            app::app_quit,
            app::app_take_warnings,
            agents::agent_list,
            agents::agent_request_approval,
            agents::agent_request_session_approval,
            agents::agent_revoke,
            agents::agent_create_session,
            agents::agent_run,
            agents::agent_sessions,
            agents::agent_stop,
            agents::agent_remove,
            agents::agent_changes,
            agents::agent_read_file,
            providers::provider_list,
            providers::provider_set_credential,
            providers::provider_remove_credential,
            providers::provider_add_model,
            providers::provider_remove_model,
            mcp::mcp_list,
            mcp::mcp_add,
            mcp::mcp_update,
            mcp::mcp_set_enabled,
            mcp::mcp_remove,
            mcp::mcp_set_secret,
            mcp::mcp_remove_secret,
            terminal::terminal_create,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_ack,
            terminal::terminal_is_busy,
            terminal::terminal_close,
            workspace::workspace_open,
            workspace::workspace_open_recent,
            workspace::workspace_recent,
            workspace::workspace_forget_recent,
            workspace::workspace_set_trust,
            workspace::workspace_search,
            workspace::workspace_search_cancel,
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
                // Agents are gone; their MCP servers go with them.
                app.state::<Mcp>().stop_all();
            }
        });
}
