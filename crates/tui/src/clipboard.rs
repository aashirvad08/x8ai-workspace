//! Copying text the user selected with the mouse: to the Mac's clipboard,
//! through `pbcopy`. Nothing is ever read from the clipboard.

use std::io::Write;
use std::process::{Command, Stdio};

pub fn copy(text: &str) -> Result<(), String> {
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not run pbcopy: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("could not copy: {e}"))?;
    }
    let status = child.wait().map_err(|e| format!("could not copy: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("pbcopy failed ({status})"))
    }
}
