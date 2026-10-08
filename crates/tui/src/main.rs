//! `x8ai`: x8ai Workspace in the terminal.
//!
//! A full-screen program in the user's own terminal. It starts on the Welcome
//! screen, where `/cd`, `/new` and `/home` open a space: the folder's own
//! workspace, with a real shell in it. It shares the desktop app's spaces,
//! recent list and trust (`crates/workspace`) and runs shells the same way
//! (`crates/pty`). See docs/decisions/0020.
//!
//! Spaces live in a background `x8ai` (`server.rs`, ADR 0024); `x8ai` in a
//! terminal attaches to it (`client.rs`), so closing the terminal stops
//! nothing.

#![forbid(unsafe_code)]

mod agents;
mod app;
mod client;
mod clipboard;
mod dialog;
mod files;
mod keys;
mod layout;
mod listing;
mod mouse;
mod pane;
mod server;
mod services;
mod space;
mod spaces;
mod theme;
mod ui;
mod welcome;
mod wire;

use std::process::ExitCode;

const HELP: &str = "\
x8ai Workspace in your terminal.

Usage: x8ai [FOLDER]

  x8ai            Open the Welcome screen, or come back to x8ai as you left it.
  x8ai FOLDER     Open FOLDER as your space.
  x8ai --stop     Stop x8ai running in the background, with its shells and
                  agents (to start the new version after an update).

x8ai keeps running in the background when you close the terminal or press
Ctrl-g d: your shells and agents go on, and x8ai brings them back.

On the Welcome screen:
  /cd <folder>    open a folder as your space (a recent one by its name)
  /new <name>     a new, empty space in ~/Workspaces
  /home           the workspace with no folder open
  /name <name>    how the welcome greets you
  /share <space>  give the open space's add-ons to another space
  /detach         leave x8ai running in the background
  /quit           quit x8ai, ending its shells and agents
  Esc             go back to the open space

In a space, press Ctrl-g, then:
  t               a new tab, with a shell
  n  p  1-9       the next, the previous, or that tab
  |  -            split: a new shell to the right, or below
  arrows  o       the pane beside, or the next one
  z               the pane alone, or back with the others
  x               close the pane
  f               the file list: Enter opens a file in $EDITOR
  a               agents: start one in its own Git worktree, review what it
                  changed, stop it, remove its session; m u l choose the
                  model, MCP servers and skills of its next launch
  m               models: providers, their API keys, local models
  u               MCP servers and their secrets
  k               the catalog: agents, models, MCP servers, skills
  e               add-ons for this space's terminals
  s               scroll back through the output
  h               the Welcome screen (the space keeps running)
  d               detach: leave x8ai running in the background
  q               quit, ending every shell and agent
  ?               every key
  Ctrl-g          send Ctrl-g to the program

The mouse: click a pane or a tab, drag the line between panes, scroll with
the wheel, drag over text to copy it.

Options:
  -h, --help      Print this help
  -V, --version   Print the version
  --stop          Stop x8ai running in the background
";

fn main() -> ExitCode {
    // Run as an agent's bridge to an MCP server's socket (docs/mcp.md), or as
    // the background x8ai: before anything else, and with nothing of the
    // terminal touched.
    let mut args = std::env::args_os().skip(1);
    match args.next() {
        Some(flag) if flag == x8ai_mcp::bridge::FLAG => {
            let code = x8ai_mcp::bridge::run_with_args(args);
            return ExitCode::from(u8::try_from(code).unwrap_or(1));
        }
        Some(flag) if flag == server::FLAG && args.next().is_none() => return server::serve(),
        _ => {}
    }
    let mut folder = None;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                return ExitCode::SUCCESS;
            }
            "-V" | "--version" => {
                println!("x8ai {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "--stop" => return server::stop(),
            _ if arg.starts_with('-') => {
                eprintln!("x8ai: unknown option {arg}. See x8ai --help.");
                return ExitCode::from(2);
            }
            _ if folder.is_some() => {
                eprintln!("x8ai: one folder at a time. See x8ai --help.");
                return ExitCode::from(2);
            }
            _ => folder = Some(arg),
        }
    }
    client::run(folder)
}
