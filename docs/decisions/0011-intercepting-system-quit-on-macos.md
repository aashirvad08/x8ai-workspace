# 0011. Intercepting Quit from the Dock, logout and shutdown on macOS

**Status:** Accepted (Phase 3)

## Context

Since Phase 2 the app asks before quitting would lose unsaved editor changes
(`src-tauri/src/app.rs`). Phase 3 adds a second reason to ask: a program still
running in a terminal. The question was asked for closing the window and for the
app's own Quit menu item (⌘Q), but not for Quit from the Dock, the app switcher,
logout, restart or shutdown.

On macOS all of those arrive as a quit Apple event, which AppKit turns into
`-[NSApplicationDelegate applicationShouldTerminate:]`. That method is the only
point where an app may delay or cancel quitting. Tauri's windowing layer (tao
0.37) installs its own application delegate class, `TaoAppDelegateParent`, which
implements `applicationWillTerminate:` (so Tauri's `RunEvent::Exit` still runs and
terminal processes are still hung up) but not `applicationShouldTerminate:`. AppKit
then treats the answer as "quit now". Tauri exposes no hook for it.

## Decision

At startup (Tauri's `setup`, on the main thread) the app adds an
`applicationShouldTerminate:` method to tao's delegate class through the
Objective-C runtime (`class_addMethod`, `src-tauri/src/macos.rs`). It asks the same
question as ⌘Q (`app::must_ask`): is anything unsaved or running, and is the
frontend there to ask? If so it answers `NSTerminateCancel` and sends
`QuitRequested` to the frontend, which shows the same dialogs as ⌘Q and quits
through `app_quit` once the user confirms. Otherwise it answers
`NSTerminateNow`, and quitting proceeds as before.

- `class_addMethod` only adds; it fails if the class already has the method. If a
  future tao implements it, installation fails, the app logs that, and tao's own
  behaviour is kept. Nothing is swizzled or replaced.
- The objc2 crate used is the version tao already depends on, so no new code is
  compiled in. It is a macOS-only dependency.
- Tauri's own exit path (`app.exit`) stops the run loop rather than calling
  `-[NSApp terminate:]`, so confirming in the dialog does not re-enter the method.

## Consequences

- Quit from the Dock or the app switcher with unsaved changes or a running
  program now asks, like ⌘Q.
- **Logout, restart and shutdown** are cancelled while the question is open, and
  macOS reports that the app interrupted them. The user answers the dialog and
  logs out again. This is the standard behaviour of document-based apps.
- **What still cannot be intercepted** on any platform: Force Quit, `kill -9`,
  crashes, power loss, and the app being killed by the system (for example on low
  memory). Unsaved editor changes are lost then. Terminal processes still end,
  because the kernel hangs up a terminal when the app's side of it closes.
- If the webview is gone or never subscribed, the app quits without asking rather
  than trapping the user; there is nobody to answer.
- This depends on tao's delegate being an Objective-C class the app can extend.
  Installation is checked at startup. The behaviour itself has no automated test,
  because that needs a running AppKit application receiving a quit Apple event;
  it is checked by hand (`osascript -e 'tell application id "com.x8ai.workspace"
  to quit'` with an unsaved file sends the same event as the Dock).

## Alternatives considered

- **Replacing the application delegate** with our own that forwards to tao's.
  Rejected: tao keeps state in its delegate's instance variables and expects its
  own class, so replacing it risks breaking launching and URL handling.
- **Method swizzling** of `-[NSApplication terminate:]`. Rejected: it changes a
  system method for every caller and is harder to reason about than adding one
  delegate method that does not exist yet.
- **`NSApplicationWillTerminateNotification`.** It fires after the decision to quit
  is final, so it cannot ask anything.
- **Waiting for Tauri to add a hook.** Rejected: the gap would stay open until
  then. If Tauri or tao adds one, this module should be replaced by it.
