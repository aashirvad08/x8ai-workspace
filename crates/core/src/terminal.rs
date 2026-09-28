//! Terminal session contracts.
//!
//! A session is a PTY running a program, normally the user's login shell. The
//! webview refers to sessions only by [`SessionId`]. Output travels on a per-session
//! channel as raw bytes, interleaved in order with [`TerminalEvent`]s. Input travels
//! as a raw request body. See `docs/architecture.md`, "Terminal architecture".

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Identifies a terminal session for the lifetime of the app process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SessionId(pub u32);

/// Name of the request header that carries the [`SessionId`] on `terminal_write`,
/// whose body is the raw input bytes.
pub const SESSION_ID_HEADER: &str = "x8ai-session-id";

/// Terminal dimensions in character cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TerminalSize {
    pub cols: u16,
    pub rows: u16,
}

impl TerminalSize {
    /// Generous upper bounds. Anything larger is a bug or a hostile caller, not a
    /// real window.
    pub const MAX_COLS: u16 = 4096;
    pub const MAX_ROWS: u16 = 2048;

    pub fn is_valid(self) -> bool {
        (1..=Self::MAX_COLS).contains(&self.cols) && (1..=Self::MAX_ROWS).contains(&self.rows)
    }
}

/// Returned by `terminal_create` once the session's process is running.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TerminalInfo {
    pub id: SessionId,
    /// Absolute path of the program running in the session, e.g. `/bin/zsh`.
    pub program: String,
    /// Flow control: acknowledge processed output with `terminal_ack` whenever at
    /// least this many bytes have been rendered since the last acknowledgement. The
    /// native side pauses reading when too much output is unacknowledged.
    pub ack_bytes: u32,
}

/// Lifecycle events delivered on a session's channel, in order with its output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "type", rename_all = "camelCase")]
#[ts(export)]
pub enum TerminalEvent {
    /// The session's process exited. No further output follows.
    Exited(TerminalExit),
    /// Reading from the terminal failed. An `exited` event follows once the process
    /// is gone.
    Error { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TerminalExit {
    pub code: u32,
    /// Set when the process was terminated by a signal, e.g. `Hangup: 1`.
    pub signal: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_bounds() {
        assert!(TerminalSize { cols: 80, rows: 24 }.is_valid());
        assert!(!TerminalSize { cols: 0, rows: 24 }.is_valid());
        assert!(!TerminalSize { cols: 80, rows: 0 }.is_valid());
        assert!(
            !TerminalSize {
                cols: TerminalSize::MAX_COLS + 1,
                rows: 24
            }
            .is_valid()
        );
    }

    #[test]
    fn events_are_tagged_by_type() {
        let exited = TerminalEvent::Exited(TerminalExit {
            code: 0,
            signal: None,
        });
        assert_eq!(
            serde_json::to_value(&exited).unwrap(),
            serde_json::json!({ "type": "exited", "code": 0, "signal": null })
        );
    }

    #[test]
    fn session_ids_are_plain_numbers_on_the_wire() {
        assert_eq!(serde_json::to_string(&SessionId(7)).unwrap(), "7");
    }
}
