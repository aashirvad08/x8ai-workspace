//! The command an agent runs for a stdio MCP server: it connects its stdin and
//! stdout to the server's socket, and exits when either side ends. It knows
//! nothing about MCP and carries no configuration or secret: only a socket path.
//! The desktop app runs as the bridge with `--mcp-bridge <socket>`; tests use the
//! `x8ai-mcp-bridge` binary.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::thread;

/// The flag the desktop app takes to act as the bridge.
pub const FLAG: &str = "--mcp-bridge";

/// Bridges this process's stdin and stdout to `socket`. Returns the exit code.
pub fn run(socket: &Path) -> i32 {
    let stream = match UnixStream::connect(socket) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!(
                "x8ai MCP bridge: cannot reach the server at {}: {error}",
                socket.display()
            );
            return 1;
        }
    };
    let Ok(mut to_server) = stream.try_clone() else {
        return 1;
    };
    thread::spawn(move || {
        let mut stdin = std::io::stdin().lock();
        let mut buf = [0u8; 16 * 1024];
        while let Ok(n) = stdin.read(&mut buf) {
            if n == 0 || to_server.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        let _ = to_server.shutdown(Shutdown::Write);
    });
    let mut from_server = stream;
    let mut stdout = std::io::stdout().lock();
    let mut buf = [0u8; 16 * 1024];
    loop {
        match from_server.read(&mut buf) {
            Ok(0) | Err(_) => return 0,
            Ok(n) => {
                if stdout
                    .write_all(&buf[..n])
                    .and_then(|()| stdout.flush())
                    .is_err()
                {
                    return 0;
                }
            }
        }
    }
}

/// `<socket>`: the arguments after the program name (and after the flag, for the
/// desktop app).
pub fn run_with_args(mut args: impl Iterator<Item = OsString>) -> i32 {
    match (args.next(), args.next()) {
        (Some(socket), None) => run(Path::new(&socket)),
        _ => {
            eprintln!("usage: x8ai-mcp-bridge <socket>");
            2
        }
    }
}
