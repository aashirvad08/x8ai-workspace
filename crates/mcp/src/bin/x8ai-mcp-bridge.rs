//! The MCP bridge as a program of its own, for tests and for a host that is not
//! the desktop app. See `x8ai_mcp::bridge`.

fn main() {
    std::process::exit(x8ai_mcp::bridge::run_with_args(std::env::args_os().skip(1)));
}
