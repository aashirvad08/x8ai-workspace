//! PTY sessions: a program running on a pseudo-terminal, normally the user's login
//! shell, with streamed output, flow control, resize and lifecycle management.
//!
//! This crate owns every terminal process the app starts. It has no Tauri
//! dependency; the desktop host adapts [`SessionEvents`] to an IPC channel. PTY
//! allocation and process spawning are delegated to `portable-pty` (ADR 0006).
//!
//! Each session runs four threads so that nothing blocks the caller:
//!
//! - **reader**: reads PTY output into a pending buffer and pauses while too much
//!   output is unacknowledged ([`FLOW_WINDOW`]);
//! - **sender**: delivers pending output and lifecycle events, in order, to
//!   [`SessionEvents`];
//! - **writer**: writes queued input to the PTY, which can block when the program
//!   is not reading;
//! - **waiter**: waits for the process to exit.

#![forbid(unsafe_code)]

mod command;
mod locale;
mod session;
mod sessions;

pub use command::{Environment, Program, user_shell};
pub use session::{ACK_BYTES, Error, FLOW_WINDOW, KILL_GRACE, Session, SessionEvents};
pub use sessions::Sessions;
