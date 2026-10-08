//! `x8ai` in the user's terminal (ADR 0024): it attaches to the background
//! `x8ai`, starting it if none runs (`server.rs`), and relays. Keys, the mouse,
//! pastes and the terminal's size go there; what to draw comes back and is
//! written to the terminal as it is. Closing the terminal ends only this
//! process: the spaces, shells and agents keep running.

use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, IsTerminal, Write, stdout};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crossterm::cursor::{SetCursorStyle, Show};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste};
use crossterm::execute;
use crossterm::style::Print;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};

use crate::server::{self, Paths};
use crate::theme;
use crate::welcome;
use crate::wire::{self, Exit, Hello, PROTOCOL, ToClient, ToServer};

/// Set in the background `x8ai`'s environment, so every program it starts
/// has it: `x8ai` in one of its panes would attach to itself.
const INSIDE: &str = "X8AI_INSIDE";

/// How long the background `x8ai` may take to start.
const START_TIMEOUT: Duration = Duration::from_secs(10);

/// Mouse reports: presses and releases (1000), drags (1002), in SGR form
/// (1006). Not every movement (1003), which would wake x8ai at each one.
const MOUSE_ON: &str = "\x1b[?1000h\x1b[?1002h\x1b[?1006h";
const MOUSE_OFF: &str = "\x1b[?1006l\x1b[?1002l\x1b[?1000l";

pub fn run(folder: Option<String>) -> ExitCode {
    if !io::stdin().is_terminal() || !stdout().is_terminal() {
        eprintln!("x8ai: needs a terminal: run it in Terminal, iTerm2, Ghostty or the like.");
        return ExitCode::FAILURE;
    }
    if std::env::var_os(INSIDE).is_some() {
        eprintln!("x8ai: this terminal is inside x8ai already. Ctrl-g h shows its Welcome.");
        return ExitCode::FAILURE;
    }
    let home = std::env::home_dir().unwrap_or_else(|| "/".into());
    let paths = Paths::new(&home);
    let stream = match connect(&paths, &home) {
        Ok(stream) => stream,
        Err(error) => {
            eprintln!("x8ai: {error}");
            return ExitCode::FAILURE;
        }
    };
    match attach(stream, folder, &home) {
        Ok(Some(Exit { message, code })) => {
            if let Some(message) = message {
                if code == 0 {
                    println!("{message}");
                } else {
                    eprintln!("x8ai: {message}");
                }
            }
            ExitCode::from(code)
        }
        Ok(None) => {
            eprintln!(
                "x8ai: the background x8ai stopped unexpectedly. What it said is in {}.",
                welcome::tilde(&paths.log, &home)
            );
            ExitCode::FAILURE
        }
        Err(error) => {
            eprintln!("x8ai: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Connects to the background `x8ai`, starting one if none answers.
fn connect(paths: &Paths, home: &Path) -> io::Result<UnixStream> {
    if let Ok(stream) = UnixStream::connect(&paths.socket) {
        return Ok(stream);
    }
    paths.make()?;
    let log = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&paths.log)?;
    let deadline = Instant::now() + START_TIMEOUT;
    let mut child: Option<Child> = None;
    loop {
        if let Ok(stream) = UnixStream::connect(&paths.socket) {
            return Ok(stream);
        }
        let start = match child.as_mut().map(Child::try_wait).transpose()? {
            None => true,
            Some(None) => false,
            // Another one held the lock, and may be ending: try again.
            Some(Some(status)) if status.success() => true,
            Some(Some(status)) => {
                return Err(io::Error::other(format!(
                    "could not start the background x8ai ({status}). What it said is in {}.",
                    welcome::tilde(&paths.log, home)
                )));
            }
        };
        if Instant::now() >= deadline {
            return Err(io::Error::other(format!(
                "the background x8ai did not start in time. What it said is in {}.",
                welcome::tilde(&paths.log, home)
            )));
        }
        if start {
            child = Some(spawn(home, &log)?);
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Starts the background `x8ai`: this program, with nothing of the terminal.
/// It takes this environment, which agents and shells get (ADR 0022).
fn spawn(home: &Path, log: &File) -> io::Result<Child> {
    Command::new(std::env::current_exe()?)
        .arg(server::FLAG)
        .env(INSIDE, "1")
        .current_dir(home)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log.try_clone()?)
        .spawn()
}

/// Says hello, takes over the terminal, and relays until told to end.
/// `None` when the background `x8ai` went away without a word.
fn attach(stream: UnixStream, folder: Option<String>, home: &Path) -> io::Result<Option<Exit>> {
    let hello = Hello {
        protocol: PROTOCOL,
        version: env!("CARGO_PKG_VERSION").to_owned(),
        cwd: std::env::current_dir().unwrap_or_else(|_| home.to_owned()),
        folder,
        size: crossterm::terminal::size().unwrap_or((80, 24)),
        truecolor: theme::truecolor_here(),
    };
    let mut writer = stream.try_clone()?;
    wire::send(&mut writer, &ToServer::Hello(hello))?;

    enter()?;
    // Reading the terminal on a thread of its own; if that fails, this
    // terminal lets go, and everything keeps running.
    let failed: Arc<Mutex<Option<String>>> = Arc::default();
    {
        let failed = Arc::clone(&failed);
        thread::spawn(move || {
            loop {
                match event::read() {
                    Ok(event) => {
                        if wire::send(&mut writer, &ToServer::Input(event)).is_err() {
                            return;
                        }
                    }
                    Err(error) => {
                        *failed.lock().unwrap_or_else(|e| e.into_inner()) = Some(error.to_string());
                        let _ = writer.shutdown(std::net::Shutdown::Both);
                        return;
                    }
                }
            }
        });
    }
    let mut reader = BufReader::new(stream);
    let mut out = stdout().lock();
    let result = loop {
        match wire::receive_from_server(&mut reader) {
            Ok(Some(ToClient::Draw(bytes))) => {
                if let Err(error) = out.write_all(&bytes).and_then(|()| out.flush()) {
                    break Err(error);
                }
            }
            Ok(Some(ToClient::Exit(exit))) => break Ok(Some(exit)),
            Ok(None) | Err(_) => break Ok(None),
        }
    };
    drop(out);
    leave();
    if let Some(error) = failed.lock().unwrap_or_else(|e| e.into_inner()).take() {
        return Err(io::Error::other(format!(
            "could not read the terminal: {error}. x8ai keeps running: run x8ai to come back to it."
        )));
    }
    result
}

/// Takes over the terminal: raw mode, the alternate screen, pastes marked as
/// pastes, and the mouse. A panic gives it back first, so its message is readable.
fn enter() -> io::Result<()> {
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
    )
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
