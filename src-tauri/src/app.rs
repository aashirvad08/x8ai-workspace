//! App lifecycle: quitting without losing work.
//!
//! Quitting asks first when it would lose something: unsaved editor changes
//! (which the frontend reports) or a program running in a terminal (which the
//! native side sees). Then closing the window, choosing Quit, and on macOS also
//! Quit from the Dock, logout and shutdown, send `AppEvent::QuitRequested`, and
//! nothing closes until the frontend calls `app_quit`. If nothing would be lost,
//! or the frontend never subscribed, quitting is immediate, so a frontend that
//! hangs cannot trap the user.
//!
//! Cannot be intercepted on any platform: Force Quit, `kill -9`, crashes and
//! power loss. Terminal processes still end then, because the kernel hangs up
//! their terminals when the app's end of the PTY closes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use x8ai_core::app::AppEvent;

use crate::terminal::Terminals;
use crate::workspace::Workspaces;

#[derive(Default)]
pub struct AppState {
    unsaved: AtomicBool,
    quitting: AtomicBool,
    events: Mutex<Option<Channel<AppEvent>>>,
}

impl AppState {
    /// Asks the frontend to confirm quitting.
    pub fn ask(&self) {
        if let Some(events) = self.lock_events().as_ref() {
            // If the page is gone, the next reload resets this state.
            let _ = events.send(AppEvent::QuitRequested);
        }
    }

    pub fn is_quitting(&self) -> bool {
        self.quitting.load(Ordering::SeqCst)
    }

    /// Called when the page reloads: its unsaved state and subscription are gone.
    pub fn reset(&self) {
        self.unsaved.store(false, Ordering::SeqCst);
        self.lock_events().take();
    }

    fn lock_events(&self) -> std::sync::MutexGuard<'_, Option<Channel<AppEvent>>> {
        self.events.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Whether a close or quit must wait for the frontend: something would be lost,
/// and the frontend is there to ask.
pub fn must_ask(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    !state.is_quitting()
        && state.lock_events().is_some()
        && (state.unsaved.load(Ordering::SeqCst) || app.state::<Terminals>().any_busy())
}

/// A quit request from the Quit menu item (Cmd+Q) or, on macOS, from the system.
pub fn quit_requested(app: &AppHandle) {
    if must_ask(app) {
        app.state::<AppState>().ask();
    } else {
        app.exit(0);
    }
}

#[tauri::command]
pub fn app_subscribe(events: Channel<AppEvent>, state: State<'_, AppState>) {
    *state.lock_events() = Some(events);
}

#[tauri::command]
pub fn app_set_unsaved_changes(unsaved: bool, state: State<'_, AppState>) {
    state.unsaved.store(unsaved, Ordering::SeqCst);
}

/// Quits now. The frontend calls this once the user has confirmed.
#[tauri::command]
pub fn app_quit(app: AppHandle, state: State<'_, AppState>) {
    state.quitting.store(true, Ordering::SeqCst);
    app.exit(0);
}

/// Native problems found at startup (for example an unreadable settings file),
/// for the frontend to show once.
#[tauri::command]
pub fn app_take_warnings(workspaces: State<'_, Workspaces>) -> Vec<String> {
    workspaces.take_warnings()
}
