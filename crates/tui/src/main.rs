//! `x8ai`: x8ai Workspace in the terminal.
//!
//! A full-screen program in the user's own terminal. It starts on the Welcome
//! screen, where `/cd`, `/new` and `/home` open a space: the folder's own
//! workspace, with a real shell in it. It shares the desktop app's spaces,
//! recent list and trust (`crates/workspace`) and runs shells the same way
//! (`crates/pty`). See docs/decisions/0020.

#![forbid(unsafe_code)]

mod agents;
mod app;
mod clipboard;
mod dialog;
mod files;
mod keys;
mod layout;
mod listing;
mod mouse;
mod pane;
mod services;
mod space;
mod spaces;
mod theme;
mod ui;
mod welcome;

use std::io::{IsTerminal, Write, stdout};
use std::process::ExitCode;
use std::sync::mpsc;

use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::cursor::{SetCursorStyle, Show};
use ratatui::crossterm::event::{DisableBracketedPaste, EnableBracketedPaste};
use ratatui::crossterm::execute;
use ratatui::crossterm::style::Print;
use ratatui::crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::{DefaultTerminal, Terminal};

use crate::app::App;
use crate::spaces::Spaces;

const HELP: &str = "\
x8ai Workspace in your terminal.

Usage: x8ai [FOLDER]

  x8ai            Open the Welcome screen.
  x8ai FOLDER     Open FOLDER as your space.

On the Welcome screen:
  /cd <folder>    open a folder as your space (a recent one by its name)
  /new <name>     a new, empty space in ~/Workspaces
  /home           the workspace with no folder open
  /name <name>    how the welcome greets you
  /share <space>  give the open space's add-ons to another space
  /quit           leave x8ai
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
  q               quit
  ?               every key
  Ctrl-g          send Ctrl-g to the program

The mouse: click a pane or a tab, drag the line between panes, scroll with
the wheel, drag over text to copy it.

Options:
  -h, --help      Print this help
  -V, --version   Print the version
";

fn main() -> ExitCode {
    // Run as an agent's bridge to an MCP server's socket (docs/mcp.md): before
    // anything else, and with nothing of the terminal touched.
    let mut args = std::env::args_os().skip(1);
    if args.next().is_some_and(|a| a == x8ai_mcp::bridge::FLAG) {
        let code = x8ai_mcp::bridge::run_with_args(args);
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
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
    if !std::io::stdin().is_terminal() || !stdout().is_terminal() {
        eprintln!("x8ai: needs a terminal: run it in Terminal, iTerm2, Ghostty or the like.");
        return ExitCode::FAILURE;
    }
    match run(folder.as_deref()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("x8ai: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(folder: Option<&str>) -> std::io::Result<()> {
    let home = std::env::home_dir().unwrap_or_else(|| "/".into());
    let cwd = std::env::current_dir().unwrap_or_else(|_| home.clone());
    let spaces = Spaces::new(spaces::data_dir(&home), home);

    let mut terminal = enter()?;
    let size = terminal.size()?;
    let (tx, rx) = mpsc::channel();
    app::read_input(tx.clone());
    let mut app = App::new(spaces, cwd, tx, (size.width, size.height));
    if let Some(folder) = folder {
        app.open_at_start(folder);
    }
    let result = app::run(&mut terminal, &mut app, &rx);
    leave();
    app.shut_down();
    result?;
    app.failure
        .map_or(Ok(()), |failure| Err(std::io::Error::other(failure)))
}

/// Mouse reports: presses and releases (1000), drags (1002), in SGR form
/// (1006). Not every movement (1003), which would wake x8ai at each one.
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";

/// Takes over the terminal: raw mode, the alternate screen, pastes marked as
/// pastes, and the mouse. A panic gives it back first, so its message is readable.
fn enter() -> std::io::Result<DefaultTerminal> {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave();
        previous(info);
    }));
    enable_raw_mode()?;
    execute!(
        stdout(),
        EnterAlternateScreen,
        EnableBracketedPaste,
        Print(MOUSE_ON)
    )?;
    Terminal::new(CrosstermBackend::new(stdout()))
}

/// Gives the terminal back as it was.
fn leave() {
    let _ = disable_raw_mode();
    let _ = execute!(
        stdout(),
        Print(MOUSE_OFF),
        DisableBracketedPaste,
        SetCursorStyle::DefaultUserShape,
        LeaveAlternateScreen,
        Show
    );
    let _ = stdout().flush();
}
