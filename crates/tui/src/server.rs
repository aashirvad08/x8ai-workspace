//! The background `x8ai` (ADR 0024). It holds the spaces, their shells and
//! agents, and draws for the one terminal attached to it (`client.rs`). It
//! outlives that terminal: closing it, or Ctrl-g d, leaves everything running,
//! and the next `x8ai` attaches again, from any terminal.
//!
//! It is the `x8ai` binary, started by the first `x8ai` with [`FLAG`], in a
//! session of its own so no terminal's hangup reaches it. One runs per user:
//! it holds a lock in `~/.x8ai/server/`, a folder only the user can open,
//! listens on a socket there, and talks only to the user's own processes. It
//! ends when the user quits, when `x8ai --stop` asks, or once no terminal is
//! attached and no space has anything open.

use std::fs::{self, DirBuilder, File, OpenOptions, Permissions};
use std::io::{self, BufReader};
use std::net::Shutdown;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::fcntl::{Flock, FlockArg};
use nix::sys::signal::{Signal, kill};
use nix::unistd::Pid;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::app::{App, Canvas, Msg, Sink};
use crate::spaces::{self, Spaces};
use crate::ui;
use crate::wire::{self, Exit, Hello, PROTOCOL, ToClient, ToServer};

/// The argument that starts the background `x8ai`. Not for users: `x8ai`
/// starts it when none runs.
pub const FLAG: &str = "--server";

/// The longest the loop handles messages before drawing, so a flood of output
/// still shows as it arrives.
const FRAME_BUDGET: Duration = Duration::from_millis(16);

/// How long it stays with nothing open and no terminal attached: long enough
/// for the `x8ai` that started it to attach, or for the user to come back.
const LINGER: Duration = Duration::from_secs(10);

/// Draws waiting for a terminal that reads slowly. When they are all waiting,
/// the next is dropped and the screen drawn whole once it catches up.
const QUEUE: usize = 64;

/// How long a terminal may take to read what was sent before it is let go.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

/// Where the background `x8ai` keeps its socket, lock, process id and log.
pub struct Paths {
    pub dir: PathBuf,
    pub socket: PathBuf,
    lock: PathBuf,
    pid: PathBuf,
    pub log: PathBuf,
}

impl Paths {
    pub fn new(home: &Path) -> Self {
        let dir = home.join(".x8ai").join("server");
        Self {
            socket: dir.join("x8ai.sock"),
            lock: dir.join("lock"),
            pid: dir.join("pid"),
            log: dir.join("log"),
            dir,
        }
    }

    /// Makes the folder, the user's alone (0700); refuses one that is not
    /// theirs.
    pub fn make(&self) -> io::Result<()> {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.dir)?;
        let meta = fs::metadata(&self.dir)?;
        if meta.uid() != nix::unistd::getuid().as_raw() {
            return Err(io::Error::other(format!(
                "{} belongs to another user",
                self.dir.display()
            )));
        }
        if meta.mode() & 0o077 != 0 {
            fs::set_permissions(&self.dir, Permissions::from_mode(0o700))?;
        }
        Ok(())
    }

    /// The background `x8ai`'s process id, if one runs: whoever holds the lock.
    pub fn running(&self) -> io::Result<Option<Pid>> {
        let file = match File::open(&self.lock) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(_free) => Ok(None),
            Err((_, Errno::EWOULDBLOCK)) => {
                let pid = fs::read_to_string(&self.pid)
                    .ok()
                    .and_then(|text| text.trim().parse::<i32>().ok())
                    .ok_or_else(|| io::Error::other("x8ai is starting: try again"))?;
                Ok(Some(Pid::from_raw(pid)))
            }
            Err((_, errno)) => Err(errno.into()),
        }
    }

    /// Takes the lock, or `None` when another background `x8ai` holds it.
    fn lock(&self) -> io::Result<Option<Flock<File>>> {
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(&self.lock)?;
        match Flock::lock(file, FlockArg::LockExclusiveNonblock) {
            Ok(lock) => Ok(Some(lock)),
            Err((_, Errno::EWOULDBLOCK)) => Ok(None),
            Err((_, errno)) => Err(errno.into()),
        }
    }
}

/// Which connection something came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientId(u64);

/// What reaches the loop from terminals.
pub enum ClientEvent {
    Hello(ClientId, Hello, UnixStream),
    Input(ClientId, crossterm::event::Event),
    Gone(ClientId),
}

/// `x8ai --server`: runs until the user quits, `x8ai --stop` asks, or nothing
/// is left to keep.
pub fn serve() -> ExitCode {
    // A session of its own: the terminal that started it can close.
    let _ = nix::unistd::setsid();
    let home = std::env::home_dir().unwrap_or_else(|| "/".into());
    let paths = Paths::new(&home);
    if let Err(error) = paths.make() {
        eprintln!("x8ai: {}: {error}", paths.dir.display());
        return ExitCode::FAILURE;
    }
    let _lock = match paths.lock() {
        Ok(Some(lock)) => lock,
        // Another one runs; the terminal attaches to it.
        Ok(None) => return ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("x8ai: could not lock {}: {error}", paths.dir.display());
            return ExitCode::FAILURE;
        }
    };
    // A socket left by one that crashed: the lock says it is gone.
    let _ = fs::remove_file(&paths.socket);
    let listener = match UnixListener::bind(&paths.socket) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!(
                "x8ai: could not listen at {}: {error}",
                paths.socket.display()
            );
            return ExitCode::FAILURE;
        }
    };
    let _ = fs::set_permissions(&paths.socket, Permissions::from_mode(0o600));
    if let Err(error) = fs::write(&paths.pid, std::process::id().to_string()) {
        eprintln!("x8ai: could not write {}: {error}", paths.pid.display());
        return ExitCode::FAILURE;
    }

    let (tx, rx) = mpsc::channel();
    if let Err(error) = watch_signals(tx.clone()) {
        eprintln!("x8ai: could not watch for signals: {error}");
        return ExitCode::FAILURE;
    }
    accept(listener, tx.clone());
    let spaces = Spaces::new(spaces::data_dir(&home), home.clone());
    let mut app = App::new(spaces, home, tx, (80, 24));
    let result = run(&mut app, &rx);
    // Everything ends before the terminal is told, so that when `x8ai` returns
    // to the shell, its programs have.
    app.shut_down();
    let _ = fs::remove_file(&paths.socket);
    let _ = fs::remove_file(&paths.pid);
    match result {
        Ok((attached, ended)) => {
            let failure = app.failure.take();
            let code = u8::from(failure.is_some());
            let message = failure.or(ended);
            if let Some(client) = attached {
                client.exit(message, code, true);
            }
            ExitCode::from(code)
        }
        Err(error) => {
            eprintln!("x8ai: {error}");
            ExitCode::FAILURE
        }
    }
}

/// `x8ai --stop`: ends the background `x8ai`, with its shells and agents.
pub fn stop() -> ExitCode {
    let home = std::env::home_dir().unwrap_or_else(|| "/".into());
    let paths = Paths::new(&home);
    let pid = match paths.running() {
        Ok(Some(pid)) => pid,
        Ok(None) => {
            println!("x8ai is not running in the background.");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("x8ai: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = kill(pid, Signal::SIGTERM) {
        eprintln!("x8ai: could not stop it (process {pid}): {error}");
        return ExitCode::FAILURE;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if matches!(paths.running(), Ok(None)) {
            println!("x8ai is stopped: its shells and agents have ended.");
            return ExitCode::SUCCESS;
        }
        thread::sleep(Duration::from_millis(50));
    }
    eprintln!("x8ai: it is still ending (process {pid}).");
    ExitCode::FAILURE
}

/// SIGTERM and SIGINT end it as quitting does; SIGHUP is ignored, as it has no
/// terminal to lose.
fn watch_signals(tx: Sender<Msg>) -> io::Result<()> {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    let mut signals = signal_hook::iterator::Signals::new([SIGTERM, SIGINT, SIGHUP])?;
    thread::spawn(move || {
        for signal in signals.forever() {
            if signal != SIGHUP && tx.send(Msg::Terminate).is_err() {
                return;
            }
        }
    });
    Ok(())
}

/// Takes connections from the user's own processes, each read on a thread of
/// its own.
fn accept(listener: UnixListener, tx: Sender<Msg>) {
    thread::spawn(move || {
        let me = nix::unistd::getuid();
        for (n, stream) in (0u64..).zip(listener.incoming()) {
            let Ok(stream) = stream else {
                continue;
            };
            if nix::unistd::getpeereid(&stream).map(|(uid, _)| uid) != Ok(me) {
                continue;
            }
            let tx = tx.clone();
            thread::spawn(move || read_client(ClientId(n), stream, &tx));
        }
    });
}

fn read_client(id: ClientId, stream: UnixStream, tx: &Sender<Msg>) {
    let Ok(writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let Ok(Some(ToServer::Hello(hello))) = wire::receive(&mut reader) else {
        return;
    };
    if tx
        .send(Msg::Client(ClientEvent::Hello(id, hello, writer)))
        .is_err()
    {
        return;
    }
    loop {
        match wire::receive(&mut reader) {
            Ok(Some(ToServer::Input(event))) => {
                if tx.send(Msg::Client(ClientEvent::Input(id, event))).is_err() {
                    return;
                }
            }
            Ok(Some(ToServer::Hello(_))) => {}
            Ok(None) | Err(_) => {
                let _ = tx.send(Msg::Client(ClientEvent::Gone(id)));
                return;
            }
        }
    }
}

/// The terminal attached: what to draw goes to it on a thread of its own, so
/// a terminal that reads slowly never holds up the programs.
struct Attached {
    id: ClientId,
    frames: SyncSender<ToClient>,
    /// To let it go when it cannot take more.
    stream: UnixStream,
    writer: JoinHandle<()>,
}

impl Attached {
    fn new(id: ClientId, stream: UnixStream) -> io::Result<Self> {
        let (frames, queue) = mpsc::sync_channel(QUEUE);
        let mut out = stream.try_clone()?;
        out.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let writer = thread::spawn(move || {
            for frame in queue {
                let exit = matches!(frame, ToClient::Exit(_));
                if wire::send_to_client(&mut out, &frame).is_err() || exit {
                    break;
                }
            }
            let _ = out.shutdown(Shutdown::Both);
        });
        Ok(Self {
            id,
            frames,
            stream,
            writer,
        })
    }

    /// Tells the terminal to give itself back to the shell and end; with
    /// `wait`, until it has been told.
    fn exit(self, message: Option<String>, code: u8, wait: bool) {
        if self
            .frames
            .try_send(ToClient::Exit(Exit { message, code }))
            .is_err()
        {
            let _ = self.stream.shutdown(Shutdown::Both);
        }
        drop(self.frames);
        if wait {
            let _ = self.writer.join();
        }
    }
}

/// A terminal of another protocol is told why it cannot attach.
fn refuse(mut stream: UnixStream, hello: &Hello) {
    let message = format!(
        "x8ai {} runs your spaces in the background, and cannot talk to x8ai {}. \
         `x8ai --stop` stops it, with its shells and agents; then run x8ai again.",
        env!("CARGO_PKG_VERSION"),
        hello.version
    );
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = wire::send_to_client(
        &mut stream,
        &ToClient::Exit(Exit {
            message: Some(message),
            code: 1,
        }),
    );
    let _ = stream.shutdown(Shutdown::Both);
}

/// The loop: draws for the terminal attached, if any, waits for something to
/// happen, handles it and everything else already waiting, and again, until
/// it is time to end. Returns the terminal still attached, and why it ended
/// when that was not the user's choice.
fn run(app: &mut App, rx: &Receiver<Msg>) -> io::Result<(Option<Attached>, Option<String>)> {
    let sink = Sink::default();
    let mut canvas: Canvas = Terminal::with_options(
        CrosstermBackend::new(sink.clone()),
        TerminalOptions {
            viewport: Viewport::Fixed(Rect::new(0, 0, 80, 24)),
        },
    )?;
    let mut state = State {
        attached: None,
        drawn: None,
        stalled: false,
        alone_since: Some(Instant::now()),
        ended: None,
    };
    loop {
        if let Some(client) = &state.attached {
            app.fit();
            app.refresh_states();
            let (cols, rows) = app.size();
            if state.drawn != Some((cols, rows)) {
                if state.drawn.is_none() {
                    // A terminal just attached: its own background (around
                    // the grid) and cursor in the app's colors.
                    let mut out = sink.clone();
                    std::io::Write::write_all(&mut out, app.theme.terminal_colors().as_bytes())?;
                }
                // A terminal just attached, or resized: drawn whole.
                canvas.resize(Rect::new(0, 0, cols, rows))?;
                state.drawn = Some((cols, rows));
            } else if state.stalled {
                canvas.clear()?;
            }
            canvas.draw(|frame| ui::draw(frame, app))?;
            app.apply_cursor(&mut canvas)?;
            let bytes = sink.take();
            state.stalled = false;
            if !bytes.is_empty() {
                match client.frames.try_send(ToClient::Draw(bytes)) {
                    Ok(()) | Err(TrySendError::Disconnected(_)) => {}
                    Err(TrySendError::Full(_)) => state.stalled = true,
                }
            }
        } else {
            app.refresh_states();
        }
        if app.quitting() || state.ended.is_some() {
            return Ok((state.attached, state.ended));
        }
        if app.take_detach() {
            if app.idle() {
                // Nothing would be kept.
                return Ok((
                    state.attached,
                    Some("Nothing is open, so x8ai ends.".into()),
                ));
            }
            if let Some(client) = state.attached.take() {
                client.exit(
                    Some(
                        "x8ai keeps running in the background. Run x8ai to come back to it.".into(),
                    ),
                    0,
                    false,
                );
                state.alone_since = Some(Instant::now());
            }
        }
        let linger = match state.alone_since {
            Some(since) if app.idle() => {
                if since.elapsed() >= LINGER {
                    return Ok((None, None));
                }
                Some(since + LINGER)
            }
            _ => None,
        };
        // Agents hung up from outside report no exit: look for it now and then.
        let settle = app
            .any_stopping()
            .then(|| Instant::now() + Duration::from_millis(100));
        let retry = state
            .stalled
            .then(|| Instant::now() + Duration::from_millis(50));
        let due = [app.sync_deadline(), settle, retry, linger]
            .into_iter()
            .flatten()
            .min();
        let first = match due {
            Some(due) => match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Ok((state.attached, None)),
            },
            None => match rx.recv() {
                Ok(msg) => Some(msg),
                Err(mpsc::RecvError) => return Ok((state.attached, None)),
            },
        };
        let started = Instant::now();
        if let Some(msg) = first {
            state.handle(app, msg);
        }
        while started.elapsed() < FRAME_BUDGET && !app.quitting() && state.ended.is_none() {
            match rx.try_recv() {
                Ok(msg) => state.handle(app, msg),
                Err(_) => break,
            }
        }
        app.end_due_syncs();
        app.settle_stopped();
    }
}

struct State {
    attached: Option<Attached>,
    /// The size last drawn at; `None` draws whole.
    drawn: Option<(u16, u16)>,
    /// The terminal could not take the last draw.
    stalled: bool,
    /// Since when no terminal is attached.
    alone_since: Option<Instant>,
    /// Why it ends, when asked to from outside.
    ended: Option<String>,
}

impl State {
    fn handle(&mut self, app: &mut App, msg: Msg) {
        match msg {
            Msg::Client(ClientEvent::Hello(id, hello, stream)) => {
                if hello.protocol != PROTOCOL {
                    refuse(stream, &hello);
                    return;
                }
                let Ok(client) = Attached::new(id, stream) else {
                    return;
                };
                // One terminal at a time: the last one opened has it.
                if let Some(old) = self.attached.replace(client) {
                    old.exit(
                        Some("x8ai is open in another terminal now.".into()),
                        0,
                        false,
                    );
                }
                self.drawn = None;
                self.stalled = false;
                self.alone_since = None;
                app.attach(&hello);
            }
            Msg::Client(ClientEvent::Input(id, event)) => {
                if self.attached.as_ref().is_some_and(|c| c.id == id) {
                    app.handle(Msg::Input(event));
                }
            }
            Msg::Client(ClientEvent::Gone(id)) => {
                // Its terminal closed: everything keeps running.
                if self.attached.as_ref().is_some_and(|c| c.id == id) {
                    self.attached = None;
                    self.alone_since = Some(Instant::now());
                }
            }
            Msg::Terminate => {
                self.ended = Some("x8ai was stopped: its shells and agents have ended.".into());
            }
            msg => app.handle(msg),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_holds_the_lock_and_says_who_it_is() {
        let temp = tempfile::tempdir().unwrap();
        let paths = Paths::new(temp.path());
        paths.make().unwrap();
        let mode = fs::metadata(&paths.dir).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o700);
        assert_eq!(paths.running().unwrap(), None);
        let lock = paths.lock().unwrap().unwrap();
        assert!(paths.lock().unwrap().is_none());
        fs::write(&paths.pid, "4242").unwrap();
        assert_eq!(paths.running().unwrap(), Some(Pid::from_raw(4242)));
        drop(lock);
        assert_eq!(paths.running().unwrap(), None);
    }
}
