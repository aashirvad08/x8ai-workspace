//! Native host for x8ai Workspace.
//!
//! This crate is the only one that knows about Tauri. It owns the window, registers
//! the IPC commands the webview may call, and holds native state such as terminal
//! sessions. Logic that does not need Tauri belongs in `crates/`.

mod commands;
mod terminal;

use tauri::webview::PageLoadEvent;
use tauri::{Manager, RunEvent};

use terminal::Terminals;

pub fn run() {
    tauri::Builder::default()
        .manage(Terminals::default())
        .on_page_load(|webview, payload| {
            // Sessions belong to the page that created them. A reload starts a new
            // page, so hang up the old page's sessions instead of leaking them.
            if payload.event() == PageLoadEvent::Started {
                webview.state::<Terminals>().close_all();
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_app_info,
            terminal::terminal_create,
            terminal::terminal_write,
            terminal::terminal_resize,
            terminal::terminal_ack,
            terminal::terminal_close,
        ])
        .build(tauri::generate_context!())
        .expect("failed to start the x8ai Workspace host")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                app.state::<Terminals>().shutdown();
            }
        });
}
