use std::io::{ErrorKind, Read, Write};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use nix::errno::Errno;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use portable_pty::{ChildKiller, MasterPty, PtySize, native_pty_system};
use x8ai_core::terminal::{SessionId, TerminalExit, TerminalSize};

use crate::command::Program;

/// Most output a session holds that the consumer has not acknowledged: read but
/// not yet delivered, plus delivered but not yet acknowledged. This bounds the
/// memory held per session and how much already-produced output still renders
/// after Ctrl+C. When the window is full, reading pauses, the kernel's PTY buffer
/// fills, and the program blocks on write: backpressure reaches the producer
/// instead of piling up.
pub const FLOW_WINDOW: usize = 512 * 1024;

/// How often the consumer should acknowledge processed output (see
/// [`Session::ack`]).
pub const ACK_BYTES: u32 = 64 * 1024;

/// How long a closed session's process gets to exit after SIGHUP before its
/// process group is sent SIGKILL.
pub const KILL_GRACE: Duration = Duration::from_secs(2);

const READ_BUFFER: usize = 64 * 1024;

// Liveness: a consumer that has rendered everything leaves fewer than ACK_BYTES
// unacknowledged, and one more read must still fit, or reading would stall.
const _: () = assert!(ACK_BYTES as usize + READ_BUFFER <= FLOW_WINDOW);

/// Minimum gap between output deliveries. Output arriving within the gap goes out
/// as one batch, so a flood becomes a few large messages per frame instead of
/// thousands of tiny ones. The first delivery after a quiet period is immediate.
const MIN_SEND_INTERVAL: Duration = Duration::from_millis(4);

/// After the process exits, its output counts as drained once the PTY reports end
/// of output. A background process that keeps the terminal open prevents end of
/// output, so the reader also counts as drained once it has waited this long with
/// nothing to read, measured from the exit: output written just before the exit
/// wakes the reader well within it. The exit is reported only then, so it never
/// overtakes output.
const DRAIN_QUIET_PERIOD: Duration = Duration::from_millis(500);

/// Concurrent `openpty(3)` calls fail intermittently on macOS (observed as
/// `Unknown error: -6`), so PTYs are opened one at a time. It takes microseconds.
static OPEN_PTY: Mutex<()> = Mutex::new(());

/// Receives a session's output and lifecycle events, in order, on the session's
/// sender thread. Implementations should return quickly.
pub trait SessionEvents: Send + Sync + 'static {
    fn output(&self, bytes: Vec<u8>);
    /// An unexpected I/O failure. `exited` still follows.
    fn error(&self, message: String);
    /// The process exited. Nothing follows.
    fn exited(&self, exit: TerminalExit);
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to start the terminal: {0}")]
    Spawn(String),
    #[error("terminal size {}x{} is out of range", .0.cols, .0.rows)]
    InvalidSize(TerminalSize),
    #[error("no terminal session {}", .0.0)]
    NotFound(SessionId),
    #[error("the terminal's process has exited")]
    Exited,
    #[error("terminal I/O failed: {0}")]
    Io(String),
}

/// A running (or exited) program on a PTY.
///
/// Closing a session, or dropping the last reference to it, hangs up the terminal
/// as closing a terminal window does.
pub struct Session {
    id: SessionId,
    program: String,
    cwd: String,
    pid: Option<u32>,
    master: Mutex<Box<dyn MasterPty + Send>>,
    /// `None` once closed. Dropping the sender stops the writer thread.
    input: Mutex<Option<mpsc::Sender<Vec<u8>>>>,
    killer: Mutex<Box<dyn ChildKiller + Send + Sync>>,
    shared: Arc<Shared>,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

#[derive(Default)]
struct State {
    /// Read from the PTY, not yet delivered.
    pending: Vec<u8>,
    /// Delivered, not yet acknowledged.
    unacked: usize,
    /// The PTY reported end of output: every process holding it has gone.
    eof: bool,
    /// When the reader started its current `read` call. `None` while it is
    /// handling data or waiting for the flow window.
    reading_since: Option<Instant>,
    error: Option<String>,
    exit: Option<TerminalExit>,
    exited_at: Option<Instant>,
    closed: bool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn wait<'a>(&self, state: MutexGuard<'a, State>) -> MutexGuard<'a, State> {
        self.changed
            .wait(state)
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn wait_timeout<'a>(
        &self,
        state: MutexGuard<'a, State>,
        timeout: Duration,
    ) -> MutexGuard<'a, State> {
        self.changed
            .wait_timeout(state, timeout)
            .unwrap_or_else(PoisonError::into_inner)
            .0
    }

    fn drained(state: &State) -> bool {
        if state.eof {
            return true;
        }
        match (state.exited_at, state.reading_since) {
            // Waiting since before the exit does not count: output written just
            // before it may not have woken the reader yet.
            (Some(exited), Some(waiting)) => waiting.max(exited).elapsed() >= DRAIN_QUIET_PERIOD,
            _ => false,
        }
    }

    fn update(&self, change: impl FnOnce(&mut State)) {
        change(&mut self.lock());
        self.changed.notify_all();
    }

    fn wait_for_exit(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut state = self.lock();
        while state.exit.is_none() {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            state = self.wait_timeout(state, deadline - now);
        }
        true
    }
}

impl Session {
    pub(crate) fn spawn(
        id: SessionId,
        program: &Program,
        size: TerminalSize,
        events: Arc<dyn SessionEvents>,
    ) -> Result<Arc<Self>, Error> {
        check_size(size)?;
        // portable-pty reports `anyhow` errors; `{:#}` includes their causes.
        let spawn_error = |e: &dyn std::fmt::Display| Error::Spawn(format!("{e:#}"));

        let pair = {
            let _guard = OPEN_PTY.lock().unwrap_or_else(PoisonError::into_inner);
            native_pty_system()
                .openpty(pty_size(size))
                .map_err(|e| spawn_error(&e))?
        };
        let (command, program_path, cwd) = program.command();
        let mut child = pair
            .slave
            .spawn_command(command)
            .map_err(|e| spawn_error(&e))?;
        // Only the child may hold the slave side; if we kept it open, reading would
        // never report end of output.
        drop(pair.slave);

        let pid = child.process_id();
        let mut killer = child.clone_killer();
        let shared = Arc::new(Shared::default());
        let (input, input_rx) = mpsc::channel();

        let started = (|| {
            let reader = pair
                .master
                .try_clone_reader()
                .map_err(|e| spawn_error(&e))?;
            let writer = pair.master.take_writer().map_err(|e| spawn_error(&e))?;
            spawn_thread("reader", id, {
                let shared = shared.clone();
                move || read_loop(reader, &shared)
            })?;
            spawn_thread("sender", id, {
                let shared = shared.clone();
                move || send_loop(&shared, &*events)
            })?;
            spawn_thread("writer", id, move || write_loop(writer, input_rx))?;
            spawn_thread("waiter", id, {
                let shared = shared.clone();
                move || {
                    let exit = match child.wait() {
                        Ok(status) => TerminalExit {
                            code: status.exit_code(),
                            signal: status.signal().map(str::to_owned),
                        },
                        // The real status is unknown: report the error, then a failure.
                        Err(e) => {
                            shared.update(|s| {
                                s.error = Some(format!("could not wait for the process: {e}"))
                            });
                            TerminalExit {
                                code: 1,
                                signal: None,
                            }
                        }
                    };
                    shared.update(|s| {
                        s.exit = Some(exit);
                        s.exited_at = Some(Instant::now());
                    });
                }
            })
        })();
        if let Err(e) = started {
            shared.update(|s| s.closed = true);
            let _ = killer.kill();
            return Err(e);
        }

        Ok(Arc::new(Self {
            id,
            program: program_path,
            cwd: cwd.display().to_string(),
            pid,
            master: Mutex::new(pair.master),
            input: Mutex::new(Some(input)),
            killer: Mutex::new(killer),
            shared,
        }))
    }

    pub fn id(&self) -> SessionId {
        self.id
    }

    /// Absolute path of the program running in the session.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// Absolute path of the directory the session started in.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Queues input for the PTY. Never blocks; a program that is not reading its
    /// input only stalls the writer thread.
    pub fn write(&self, bytes: Vec<u8>) -> Result<(), Error> {
        if self.has_exited() {
            return Err(Error::Exited);
        }
        let input = self.input.lock().unwrap_or_else(PoisonError::into_inner);
        input
            .as_ref()
            .ok_or(Error::Exited)?
            .send(bytes)
            .map_err(|_| Error::Exited)
    }

    /// Resizes the terminal. The kernel sends the foreground process SIGWINCH.
    pub fn resize(&self, size: TerminalSize) -> Result<(), Error> {
        check_size(size)?;
        self.master
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .resize(pty_size(size))
            .map_err(|e| Error::Io(format!("{e:#}")))
    }

    /// Acknowledges that `bytes` of delivered output have been processed.
    pub fn ack(&self, bytes: u32) {
        self.shared
            .update(|s| s.unacked = s.unacked.saturating_sub(bytes as usize));
    }

    /// Whether a job other than the session's own process is in the terminal's
    /// foreground: `vim`, a build, a `sleep` started from the shell. An idle shell
    /// at its prompt has none. Read from the PTY (`tcgetpgrp`), so it is exact at
    /// the moment it is asked.
    pub fn has_foreground_job(&self) -> bool {
        if self.has_exited() {
            return false;
        }
        let foreground = self
            .master
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .process_group_leader();
        matches!((foreground, self.pid), (Some(group), Some(pid)) if group as u32 != pid)
    }

    pub fn has_exited(&self) -> bool {
        self.shared.lock().exit.is_some()
    }

    /// Waits up to `timeout` for the process to exit. Returns whether it did.
    pub fn wait_for_exit(&self, timeout: Duration) -> bool {
        self.shared.wait_for_exit(timeout)
    }

    /// Hangs up the terminal: stops delivering events, sends SIGHUP to the process
    /// and the terminal's foreground job (the shell forwards it to its other jobs),
    /// and sends SIGKILL to the process's group if it is still running after
    /// [`KILL_GRACE`]. Jobs the user deliberately detached from the terminal (`nohup`,
    /// `disown`) are left alone, as in any terminal. Idempotent.
    pub fn close(&self) {
        {
            let mut state = self.shared.lock();
            if state.closed {
                return;
            }
            state.closed = true;
        }
        self.shared.changed.notify_all();
        self.input
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if self.has_exited() {
            return;
        }

        let foreground = self
            .master
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .process_group_leader();
        // portable-pty's killer sends SIGHUP.
        let _ = self
            .killer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .kill();
        if let Some(group) = foreground.filter(|&g| Some(g as u32) != self.pid) {
            let _ = killpg(Pid::from_raw(group), Signal::SIGHUP);
        }

        let (shared, pid) = (self.shared.clone(), self.pid);
        // If the thread cannot be spawned the process is still hung up; it just
        // will not be force-killed.
        let _ = spawn_thread("reaper", self.id, move || {
            if !shared.wait_for_exit(KILL_GRACE) {
                kill_group(pid);
            }
        });
    }

    /// Sends SIGKILL to the process's group immediately. Used at app shutdown, when
    /// there is no time for [`KILL_GRACE`].
    pub(crate) fn kill_now(&self) {
        if !self.has_exited() {
            kill_group(self.pid);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.close();
    }
}

fn kill_group(pid: Option<u32>) {
    // The session's process is a session leader, so its pid is also its process
    // group id.
    if let Some(pid) = pid.and_then(|p| i32::try_from(p).ok()) {
        let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
    }
}

fn read_loop(mut reader: Box<dyn Read + Send>, shared: &Shared) {
    let mut buf = vec![0; READ_BUFFER];
    loop {
        shared.lock().reading_since = Some(Instant::now());
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            // How a PTY reports that every process holding it has gone.
            Err(e) if e.raw_os_error() == Some(Errno::EIO as i32) => break,
            Err(e) => {
                shared.update(|s| s.error = Some(format!("reading terminal output failed: {e}")));
                break;
            }
        };
        let mut state = shared.lock();
        state.reading_since = None;
        // Wait until this whole read fits, so the window is never exceeded.
        while !state.closed && state.pending.len() + state.unacked + n > FLOW_WINDOW {
            state = shared.wait(state);
        }
        // A closed session's output is discarded, but reading goes on until the
        // end of output. macOS makes the last process to close a terminal wait
        // until its unread output drains, so a shell that wrote anything after
        // the hangup (its prompt, readline's cleanup) could otherwise never
        // finish exiting, even after SIGKILL, while the PTY stays open.
        if state.closed {
            continue;
        }
        state.pending.extend_from_slice(&buf[..n]);
        drop(state);
        shared.changed.notify_all();
    }
    shared.update(|s| {
        s.reading_since = None;
        s.eof = true;
    });
}

fn send_loop(shared: &Shared, events: &dyn SessionEvents) {
    loop {
        let mut state = shared.lock();
        loop {
            if state.closed {
                return;
            }
            if !state.pending.is_empty() {
                break;
            }
            if let Some(message) = state.error.take() {
                drop(state);
                events.error(message);
                state = shared.lock();
                continue;
            }
            match &state.exit {
                Some(exit) if Shared::drained(&state) => {
                    let exit = exit.clone();
                    drop(state);
                    events.exited(exit);
                    return;
                }
                Some(_) => state = shared.wait_timeout(state, DRAIN_QUIET_PERIOD),
                None => state = shared.wait(state),
            }
        }
        // Moving bytes from pending to unacked leaves the flow window unchanged, so
        // the reader needs no wake-up.
        let batch = std::mem::take(&mut state.pending);
        state.unacked += batch.len();
        drop(state);
        events.output(batch);
        thread::sleep(MIN_SEND_INTERVAL);
    }
}

fn write_loop(mut writer: Box<dyn Write + Send>, input: mpsc::Receiver<Vec<u8>>) {
    for bytes in input {
        // A failed write means the terminal is gone; the waiter reports the exit.
        if writer
            .write_all(&bytes)
            .and_then(|()| writer.flush())
            .is_err()
        {
            break;
        }
    }
}

fn spawn_thread(
    role: &str,
    id: SessionId,
    body: impl FnOnce() + Send + 'static,
) -> Result<(), Error> {
    thread::Builder::new()
        .name(format!("pty-{}-{role}", id.0))
        .spawn(body)
        .map(drop)
        .map_err(|e| Error::Spawn(format!("could not start the {role} thread: {e}")))
}

fn check_size(size: TerminalSize) -> Result<(), Error> {
    if size.is_valid() {
        Ok(())
    } else {
        Err(Error::InvalidSize(size))
    }
}

fn pty_size(size: TerminalSize) -> PtySize {
    PtySize {
        rows: size.rows,
        cols: size.cols,
        pixel_width: 0,
        pixel_height: 0,
    }
}
