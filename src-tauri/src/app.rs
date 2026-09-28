//! App lifecycle: quitting without losing unsaved work.
//!
//! The frontend reports whether it has unsaved changes. While it does, closing the
//! window or choosing Quit asks the frontend first (`AppEvent::QuitRequested`), and
//! nothing closes until it calls `app_quit`. While it has none, or if it never
//! subscribed, quitting is immediate, so a frontend that hangs cannot trap the user.
//!
//! Not intercepted: Quit from the Dock and system logout. They terminate the app
//! directly, without an event that can be prevented.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};
use x8ai_core::app::AppEvent;

#[derive(Default)]
pub struct AppState {
    unsaved: AtomicBool,
    quitting: AtomicBool,
    events: Mutex<Option<Channel<AppEvent>>>,
}

impl AppState {
    /// Whether a close or quit must wait for the frontend.
    pub fn must_ask(&self) -> bool {
        !self.quitting.load(Ordering::SeqCst)
            && self.unsaved.load(Ordering::SeqCst)
            && self.lock_events().is_some()
    }

    /// Asks the frontend to confirm quitting.
    pub fn ask(&self) {
        if let Some(events) = self.lock_events().as_ref() {
            // If the page is gone, the next reload resets this state.
            let _ = events.send(AppEvent::QuitRequested);
        }
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

/// The Quit menu item (Cmd+Q).
pub fn quit_requested(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.must_ask() {
        state.ask();
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
