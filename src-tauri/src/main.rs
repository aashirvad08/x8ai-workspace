fn main() {
    // Run as an MCP bridge (docs/mcp.md): before anything of the app starts, so it
    // is a plain command connecting its stdin and stdout to one socket.
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|a| a == x8ai_mcp::bridge::FLAG) {
        std::process::exit(x8ai_mcp::bridge::run_with_args(args));
    }
    x8ai_desktop::run();
}
