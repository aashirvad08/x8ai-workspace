//! Terminal commands: the IPC face of `x8ai-pty`.
//!
//! The webview can only start the user's login shell, never a program of its
//! choosing, in a directory it cannot choose either: the open workspace's root, or
//! the home directory. It refers to sessions by id afterwards. Every argument is
//! validated here or in `x8ai-pty`.

use std::sync::Arc;
use std::time::Duration;

use tauri::State;
use tauri::ipc::{Channel, InvokeBody, InvokeResponseBody, Request};
use x8ai_core::error::{CommandError, ErrorCode};
use x8ai_core::terminal::{
    SESSION_ID_HEADER, SessionId, TerminalEvent, TerminalExit, TerminalInfo, TerminalSize,
};
use x8ai_pty::{ACK_BYTES, Program, SessionEvents, Sessions};

use crate::workspace::Workspaces;

/// How long terminal processes get to exit after hangup when the app quits.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// The app's terminal sessions. Managed Tauri state.
#[derive(Default)]
pub struct Terminals(Sessions);

impl Terminals {
    /// Hangs up every session. Called when the page that owns them (re)loads.
    /// Sessions belong to the app's only window; a second window would need them
    /// scoped to the webview that created them.
    pub fn close_all(&self) {
        self.0.close_all();
    }

    /// Called on app exit: hangs up every session and kills what does not exit.
    pub fn shutdown(&self) {
        self.0.shutdown(SHUTDOWN_GRACE);
    }

    /// Whether quitting now would end a running program in some terminal.
    pub fn any_busy(&self) -> bool {
        self.0.any_foreground_job()
    }
}

/// Delivers a session's output as raw bytes and its lifecycle events as JSON, on one
/// channel, so the webview receives them in order.
struct ChannelEvents(Channel);

impl ChannelEvents {
    fn send_event(&self, event: &TerminalEvent) {
        match serde_json::to_string(event) {
            Ok(json) => self.send(InvokeResponseBody::Json(json)),
            Err(e) => eprintln!("could not serialize terminal event {event:?}: {e}"),
        }
    }

    fn send(&self, body: InvokeResponseBody) {
        // Sending fails only when the page that created the session is gone. Its
        // sessions are closed when the page reloads (see `lib.rs`).
        let _ = self.0.send(body);
    }
}

impl SessionEvents for ChannelEvents {
    fn output(&self, bytes: Vec<u8>) {
        self.send(InvokeResponseBody::Raw(bytes));
    }

    fn error(&self, message: String) {
        self.send_event(&TerminalEvent::Error { message });
    }

    fn exited(&self, exit: TerminalExit) {
        self.send_event(&TerminalEvent::Exited(exit));
    }
}

/// Starts the user's login shell in a new session, in the open workspace's root
/// (or the home directory). Output and lifecycle events arrive on `events`.
/// Sessions keep their directory when the workspace changes later.
#[tauri::command]
pub async fn terminal_create(
    size: TerminalSize,
    events: Channel,
    terminals: State<'_, Terminals>,
    workspaces: State<'_, Workspaces>,
) -> Result<TerminalInfo, CommandError> {
    let program = Program::LoginShell {
        cwd: workspaces.root(),
    };
    let session = terminals
        .0
        .spawn(&program, size, Arc::new(ChannelEvents(events)))
        .map_err(command_error)?;
    Ok(TerminalInfo {
        id: session.id(),
        program: session.program().to_owned(),
        cwd: session.cwd().to_owned(),
        ack_bytes: ACK_BYTES,
    })
}

/// Input is the raw request body, so any bytes, including non-UTF-8 mouse reports,
/// reach the PTY unchanged. The session id travels in a header.
#[tauri::command]
pub fn terminal_write(
    request: Request<'_>,
    terminals: State<'_, Terminals>,
) -> Result<(), CommandError> {
    let id = request
        .headers()
        .get(SESSION_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .map(SessionId)
        .ok_or_else(|| invalid_input(format!("missing or invalid {SESSION_ID_HEADER} header")))?;
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err(invalid_input("terminal input must be sent as raw bytes"));
    };
    terminals
        .0
        .get(id)
        .and_then(|session| session.write(bytes.clone()))
        .map_err(command_error)
}

#[tauri::command]
pub fn terminal_resize(
    id: SessionId,
    size: TerminalSize,
    terminals: State<'_, Terminals>,
) -> Result<(), CommandError> {
    terminals
        .0
        .get(id)
        .and_then(|session| session.resize(size))
        .map_err(command_error)
}

/// Flow control: the webview has rendered `bytes` more of the session's output.
#[tauri::command]
pub fn terminal_ack(
    id: SessionId,
    bytes: u32,
    terminals: State<'_, Terminals>,
) -> Result<(), CommandError> {
    terminals
        .0
        .get(id)
        .map(|session| session.ack(bytes))
        .map_err(command_error)
}

/// Whether a program is running in the session's foreground (beyond an idle
/// shell), so closing it would end that program. Asked before closing.
#[tauri::command]
pub fn terminal_is_busy(
    id: SessionId,
    terminals: State<'_, Terminals>,
) -> Result<bool, CommandError> {
    terminals
        .0
        .get(id)
        .map(|session| session.has_foreground_job())
        .map_err(command_error)
}

/// Hangs up the session and forgets it.
#[tauri::command]
pub fn terminal_close(id: SessionId, terminals: State<'_, Terminals>) -> Result<(), CommandError> {
    terminals.0.close(id).map_err(command_error)
}

fn command_error(error: x8ai_pty::Error) -> CommandError {
    use x8ai_pty::Error;
    let code = match &error {
        Error::InvalidSize(_) | Error::Exited => ErrorCode::InvalidInput,
        Error::NotFound(_) => ErrorCode::NotFound,
        Error::Spawn(_) | Error::Io(_) => ErrorCode::Internal,
    };
    CommandError::new(code, error.to_string())
}

fn invalid_input(message: impl Into<String>) -> CommandError {
    CommandError::new(ErrorCode::InvalidInput, message)
}
