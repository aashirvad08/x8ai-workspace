//! Drives the real `x8ai` binary on a PTY: its screen is read back through
//! the same emulator the panes use, and keys and mouse reports are typed into
//! it. A home folder and a data folder of its own keep it away from the user's,
//! and its background `x8ai` (`~/.x8ai/server` in that home) from theirs; it
//! is stopped when the test is done with it.

#![allow(dead_code)]

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as TermEvent, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::Processor;
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_pty::{Environment, Program, Session, SessionEvents, Sessions};

pub const TIMEOUT: Duration = Duration::from_secs(15);
pub const SIZE: TerminalSize = TerminalSize {
    cols: 100,
    rows: 30,
};
pub const CTRL_G: &str = "\x07";

enum Event {
    Output(Vec<u8>),
    Exited(TerminalExit),
}

struct Forward(Sender<Event>);

impl SessionEvents for Forward {
    fn output(&self, bytes: Vec<u8>) {
        let _ = self.0.send(Event::Output(bytes));
    }
    fn error(&self, _: String) {}
    fn exited(&self, exit: TerminalExit) {
        let _ = self.0.send(Event::Exited(exit));
    }
}

#[derive(Clone, Default)]
struct Listener(Rc<RefCell<Vec<TermEvent>>>);

impl EventListener for Listener {
    fn send_event(&self, event: TermEvent) {
        self.0.borrow_mut().push(event);
    }
}

struct Grid;

impl Dimensions for Grid {
    fn total_lines(&self) -> usize {
        usize::from(SIZE.rows)
    }
    fn screen_lines(&self) -> usize {
        usize::from(SIZE.rows)
    }
    fn columns(&self) -> usize {
        usize::from(SIZE.cols)
    }
}

/// `x8ai` running in a terminal of its own.
pub struct X8ai {
    _sessions: Sessions,
    session: Arc<Session>,
    events: Receiver<Event>,
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
    exit: Option<TerminalExit>,
    home: PathBuf,
    /// Leave the background `x8ai` running when this is dropped.
    keep_server: bool,
}

impl X8ai {
    pub fn start(home: &Path, args: &[&str]) -> Self {
        Self::start_with(home, args, &[])
    }

    /// With more variables, which replace the defaults of the same name.
    pub fn start_with(home: &Path, args: &[&str], more: &[(&str, String)]) -> Self {
        let (tx, events) = mpsc::channel();
        let sessions = Sessions::default();
        let mut env: Vec<(String, String)> = vec![
            ("HOME".into(), home.display().to_string()),
            ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
            ("SHELL".into(), "/bin/sh".into()),
            ("LANG".into(), "en_US.UTF-8".into()),
            (
                "X8AI_DATA_DIR".into(),
                home.join(".x8ai-data").display().to_string(),
            ),
        ];
        for (name, value) in more {
            env.retain(|(n, _)| n != name);
            env.push(((*name).to_owned(), value.clone()));
        }
        let program = Program::Exec {
            program: env!("CARGO_BIN_EXE_x8ai").into(),
            args: args.iter().map(Into::into).collect(),
            cwd: Some(home.to_owned()),
            env: Environment::Exactly(env),
        };
        let session = sessions
            .spawn(&program, SIZE, Arc::new(Forward(tx)))
            .unwrap();
        let listener = Listener::default();
        let term = Term::new(Config::default(), &Grid, listener.clone());
        Self {
            _sessions: sessions,
            session,
            events,
            term,
            parser: Processor::new(),
            listener,
            exit: None,
            home: home.to_owned(),
            keep_server: false,
        }
    }

    /// Closes its terminal, as closing the window does, and leaves the
    /// background `x8ai` running.
    pub fn hang_up(mut self) {
        self.keep_server = true;
        self.session.close();
        let deadline = Instant::now() + TIMEOUT;
        while !self.session.has_exited() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(self.session.has_exited(), "x8ai did not end when hung up");
    }

    pub fn keys(&self, text: &str) {
        self.session.write(text.as_bytes().to_vec()).unwrap();
    }

    /// Handles one event from `x8ai`, waiting at most until `deadline`.
    pub fn pump(&mut self, deadline: Instant) -> bool {
        match self
            .events
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(Event::Output(bytes)) => {
                self.parser.advance(&mut self.term, &bytes);
                for event in std::mem::take(&mut *self.listener.0.borrow_mut()) {
                    if let TermEvent::PtyWrite(text) = event {
                        self.keys(&text);
                    }
                }
                self.session.ack(bytes.len() as u32);
                true
            }
            Ok(Event::Exited(exit)) => {
                self.exit = Some(exit);
                true
            }
            Err(_) => false,
        }
    }

    pub fn screen(&self) -> String {
        let grid = self.term.grid();
        let mut text = String::new();
        for line in 0..grid.screen_lines() {
            for column in 0..grid.columns() {
                let cell = &grid[Point::new(Line(line as i32), Column(column))];
                if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                    text.push(cell.c);
                }
            }
            text.push('\n');
        }
        text
    }

    /// Waits until `done` holds for the screen (`what` says what that is, for
    /// the failure); returns the screen.
    pub fn wait_until(&mut self, what: &str, done: impl Fn(&str) -> bool) -> String {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let screen = self.screen();
            if done(&screen) {
                return screen;
            }
            assert!(
                self.exit.is_none() && self.pump(deadline),
                "never {what}:\n{screen}"
            );
        }
    }

    /// Waits until the screen shows `needle`; returns the screen.
    pub fn wait_for(&mut self, needle: &str) -> String {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let screen = self.screen();
            if screen.contains(needle) {
                return screen;
            }
            assert!(
                self.exit.is_none() && self.pump(deadline),
                "no {needle:?} on the screen:\n{screen}"
            );
        }
    }

    pub fn wait_for_exit(&mut self) -> TerminalExit {
        let deadline = Instant::now() + TIMEOUT;
        while self.exit.is_none() {
            assert!(self.pump(deadline), "x8ai did not quit:\n{}", self.screen());
        }
        self.exit.clone().unwrap()
    }
}

impl X8ai {
    /// Where `needle` is on the screen: its column and row, from 0.
    pub fn find(&self, needle: &str) -> Option<(usize, usize)> {
        self.screen().lines().enumerate().find_map(|(row, line)| {
            let at = line.find(needle)?;
            Some((line[..at].chars().count(), row))
        })
    }

    /// A left click at a cell (from 0), as an SGR mouse report.
    pub fn click(&self, col: usize, row: usize) {
        self.keys(&format!(
            "\x1b[<0;{};{}M\x1b[<0;{};{}m",
            col + 1,
            row + 1,
            col + 1,
            row + 1
        ));
    }

    /// A left-button drag from one cell to another, released there.
    pub fn drag(&self, from: (usize, usize), to: (usize, usize)) {
        self.keys(&format!(
            "\x1b[<0;{};{}M\x1b[<32;{};{}M\x1b[<0;{};{}m",
            from.0 + 1,
            from.1 + 1,
            to.0 + 1,
            to.1 + 1,
            to.0 + 1,
            to.1 + 1
        ));
    }

    /// The mouse wheel, one notch up or down, at a cell.
    pub fn wheel(&self, col: usize, row: usize, up: bool) {
        let button = if up { 64 } else { 65 };
        self.keys(&format!("\x1b[<{button};{};{}M", col + 1, row + 1));
    }
}

impl Drop for X8ai {
    fn drop(&mut self) {
        if !self.keep_server {
            stop_server(&self.home);
        }
    }
}

/// The background `x8ai` of `home`, while it runs.
pub fn server_pid(home: &Path) -> Option<i32> {
    let pid = std::fs::read_to_string(home.join(".x8ai/server/pid"))
        .ok()?
        .trim()
        .parse::<i32>()
        .ok()?;
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None)
        .is_ok()
        .then_some(pid)
}

/// Stops the background `x8ai` of `home`, if one runs.
pub fn stop_server(home: &Path) {
    use nix::sys::signal::{Signal, kill};
    let Some(pid) = server_pid(home) else {
        return;
    };
    let pid = nix::unistd::Pid::from_raw(pid);
    let _ = kill(pid, Signal::SIGTERM);
    let deadline = Instant::now() + Duration::from_secs(3);
    while kill(pid, None).is_ok() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    let _ = kill(pid, Signal::SIGKILL);
}

/// Waits until `done` holds, or fails saying `what` never happened.
pub fn eventually(what: &str, done: impl Fn() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !done() {
        assert!(Instant::now() < deadline, "never {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}
