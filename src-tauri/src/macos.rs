//! macOS: route system quit requests through the same check as Cmd+Q.
//!
//! Quit from the Dock, logout, restart and shutdown reach an app as
//! `-[NSApplicationDelegate applicationShouldTerminate:]`, the one place macOS
//! lets an app delay or cancel quitting. Tauri's windowing layer (tao) installs
//! an application delegate that does not implement it, so those paths quit
//! immediately, skipping the unsaved-changes question. (They do still run
//! `applicationWillTerminate:`, which ends terminal processes.)
//!
//! This adds the method to tao's delegate class at startup. When something would
//! be lost (`app::must_ask`), it cancels the termination and asks the frontend;
//! the user then quits from the dialog. For logout and shutdown this means macOS
//! reports that the app cancelled them, which is standard for apps with unsaved
//! work. Otherwise it lets the quit proceed.
//!
//! If tao ever implements the method itself, installation reports that instead of
//! replacing it.

use std::sync::OnceLock;

use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
use objc2::{class, msg_send, sel};
use tauri::{AppHandle, Manager};

use crate::app;

static APP: OnceLock<AppHandle> = OnceLock::new();

/// `NSApplicationTerminateReply` values.
const TERMINATE_CANCEL: usize = 0;
const TERMINATE_NOW: usize = 1;

/// Installs the handler. Must run on the main thread once the application
/// delegate exists, which is the case in Tauri's `setup`.
pub fn intercept_termination(handle: &AppHandle) -> Result<(), String> {
    let _ = APP.set(handle.clone());
    // SAFETY: called on the main thread (Tauri's setup hook), where AppKit may be
    // used. `sharedApplication` and `delegate` are plain getters. The delegate's
    // class is a real Objective-C class, so adding a method to it is valid. The
    // function has the signature `applicationShouldTerminate:` requires:
    // (id self, SEL _cmd, NSApplication *sender) -> NSUInteger, which is what the
    // type encoding "Q@:@" states.
    unsafe {
        let application: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        let delegate: *mut AnyObject = msg_send![application, delegate];
        let Some(delegate) = delegate.as_ref() else {
            return Err("no application delegate".into());
        };
        let class = std::ptr::from_ref::<AnyClass>(delegate.class()).cast_mut();
        let imp: Imp = std::mem::transmute::<
            extern "C-unwind" fn(&AnyObject, Sel, *mut AnyObject) -> usize,
            Imp,
        >(should_terminate);
        let added = objc2::ffi::class_addMethod(
            class,
            sel!(applicationShouldTerminate:),
            imp,
            c"Q@:@".as_ptr(),
        );
        if !added.as_bool() {
            return Err(
                "the application delegate already handles applicationShouldTerminate:".into(),
            );
        }
    }
    Ok(())
}

extern "C-unwind" fn should_terminate(
    _delegate: &AnyObject,
    _cmd: Sel,
    _sender: *mut AnyObject,
) -> usize {
    match APP.get() {
        Some(handle) if app::must_ask(handle) => {
            handle.state::<app::AppState>().ask();
            TERMINATE_CANCEL
        }
        _ => TERMINATE_NOW,
    }
}
