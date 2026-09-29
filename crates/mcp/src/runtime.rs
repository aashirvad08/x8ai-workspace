//! Stdio MCP servers, owned by the agent session they were started for.
//!
//! The agent is the MCP client, but it does not start a stdio server itself:
//! the app does, with the environment the app decides (`environment`), so no
//! agent can hand its own environment, including provider keys, to a server.
//! The agent is given a bridge instead (`bridge`): a tiny command that connects
//! its stdin and stdout to a private socket of the session, where the app starts
//! the server and connects it.
//!
//! ```text
//!  agent ──spawns──▶ bridge ⇄ ~/.x8ai/mcp/<session>/<n>.sock ⇄ app ──spawns──▶ server
//! ```
//!
//! - A server starts only when the session's agent connects: nothing runs at
//!   startup, or merely because a session exists.
//! - Only the session's agent, or a process it started, may connect (the peer's
//!   process id is checked against the agent's), and one connection at a time.
//! - A server that does not answer within the startup timeout of its first
//!   request is killed. A server started too many times in one run is not
//!   started again. Its error output is kept in a bounded buffer, redacted.
//! - The run ends when its agent's process is gone, however it ended (it exited,
//!   was stopped, its terminal closed, its workspace closed or lost trust), and
//!   when the app quits: every server's process group is killed and the sockets
//!   are removed.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::Shutdown;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::McpServerState;

use crate::approvals::MaterialTransport;
use crate::environment::ServerEnvironment;
use crate::session::{Authorized, Prepared};

/// Longest socket path the platforms accept, with room to spare.
const MAX_SOCKET_PATH: usize = 100;
const POLL: Duration = Duration::from_millis(20);
/// How often a run checks that its agent still runs.
const OWNER_POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// How long a server may take to answer the agent's first request.
    pub startup_timeout: Duration,
    /// How many times one server may be started in one run of an agent.
    pub max_starts: u32,
    /// How much of a server's error output is kept.
    pub stderr_bytes: usize,
    /// How long servers get to exit after SIGTERM before SIGKILL.
    pub stop_grace: Duration,
    /// How long a connection waits for the agent's process id to be known.
    pub owner_wait: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            startup_timeout: Duration::from_secs(30),
            max_starts: 5,
            stderr_bytes: 16 * 1024,
            stop_grace: Duration::from_secs(2),
            owner_wait: Duration::from_secs(5),
        }
    }
}

/// A stdio server to run for a session: what was approved, with its environment
/// and directory.
#[derive(Debug, Clone)]
pub struct Launch {
    pub id: IntegrationId,
    pub name: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: ServerEnvironment,
    pub cwd: PathBuf,
}

impl Launch {
    /// The launch of a prepared stdio server; `None` for an HTTP server, which
    /// the app never starts.
    pub fn new(prepared: &Prepared, env: ServerEnvironment, cwd: PathBuf) -> Option<Self> {
        match &prepared.material.transport {
            MaterialTransport::Stdio { program, args } => Some(Self {
                id: prepared.server.id.clone(),
                name: prepared.server.name.clone(),
                program: program.clone(),
                args: args.clone(),
                env,
                cwd,
            }),
            MaterialTransport::StreamableHttp { .. } => None,
        }
    }
}

/// Where the agent reaches a server: the socket its bridge connects to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub id: IntegrationId,
    pub socket: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    #[error("{0} was not approved as it would run")]
    NotAuthorized(String),
    #[error("{path}: {detail}")]
    Io { path: String, detail: String },
    #[error("the socket path {0} is too long")]
    PathTooLong(String),
}

/// The MCP servers of every agent session, keyed by session.
pub struct McpRuntime {
    dir: PathBuf,
    limits: Limits,
    runs: Arc<Mutex<HashMap<u32, Arc<Run>>>>,
    last_token: AtomicU64,
}

struct Run {
    /// Which run of the session this is.
    token: u64,
    dir: PathBuf,
    owner: Mutex<Option<u32>>,
    owner_known: Condvar,
    stopping: AtomicBool,
    slots: Vec<Arc<Slot>>,
    limits: Limits,
}

struct Slot {
    launch: Launch,
    socket: PathBuf,
    inner: Mutex<SlotInner>,
}

struct SlotInner {
    state: McpServerState,
    starts: u32,
    /// The running server's process group.
    group: Option<i32>,
    connection: Option<UnixStream>,
}

impl McpRuntime {
    /// Sockets go in `dir` (`~/.x8ai/mcp`), a directory of the app.
    pub fn new(dir: PathBuf, limits: Limits) -> Self {
        Self {
            dir,
            limits,
            runs: Arc::new(Mutex::new(HashMap::new())),
            last_token: AtomicU64::new(0),
        }
    }

    /// Removes socket directories left by an earlier run of the app that ended
    /// without cleaning up (a crash). Starts nothing.
    pub fn sweep(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let live: Vec<PathBuf> = lock(&self.runs).values().map(|r| r.dir.clone()).collect();
        for entry in entries.flatten() {
            let path = entry.path();
            let ours = entry.file_name().to_str().is_some_and(is_run_dir_name);
            if ours && !live.contains(&path) && entry.file_type().is_ok_and(|t| t.is_dir()) {
                remove_run_dir(&path);
            }
        }
    }

    /// Prepares `launches` for `session`: a socket for each, where the server is
    /// started when the session's agent connects. Replaces any earlier run of the
    /// session. Each launch must be exactly one of the authorized servers.
    pub fn start(
        &self,
        session: u32,
        authorized: &Authorized<'_>,
        launches: Vec<Launch>,
    ) -> Result<Vec<Endpoint>, RuntimeError> {
        for launch in &launches {
            let approved = authorized.servers.iter().any(|p| {
                p.server.id == launch.id
                    && p.material.transport
                        == MaterialTransport::Stdio {
                            program: launch.program.clone(),
                            args: launch.args.clone(),
                        }
            });
            if !approved {
                return Err(RuntimeError::NotAuthorized(launch.name.clone()));
            }
        }
        self.stop(session);
        let io = |path: &Path| {
            let path = path.display().to_string();
            move |e: std::io::Error| RuntimeError::Io {
                path: path.clone(),
                detail: e.to_string(),
            }
        };
        private_dir(&self.dir).map_err(io(&self.dir))?;
        let dir = self.dir.join(format!("{session}-{}", token()));
        std::fs::create_dir(&dir).map_err(io(&dir))?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).map_err(io(&dir))?;

        let mut slots = Vec::new();
        let mut listeners = Vec::new();
        for (i, launch) in launches.into_iter().enumerate() {
            let socket = dir.join(format!("{i}.sock"));
            if socket.as_os_str().len() > MAX_SOCKET_PATH {
                remove_run_dir(&dir);
                return Err(RuntimeError::PathTooLong(socket.display().to_string()));
            }
            let listener = UnixListener::bind(&socket).map_err(io(&socket))?;
            std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))
                .map_err(io(&socket))?;
            listener.set_nonblocking(true).map_err(io(&socket))?;
            listeners.push(listener);
            slots.push(Arc::new(Slot {
                launch,
                socket,
                inner: Mutex::new(SlotInner {
                    state: McpServerState::Waiting,
                    starts: 0,
                    group: None,
                    connection: None,
                }),
            }));
        }
        let run = Arc::new(Run {
            token: self.last_token.fetch_add(1, Ordering::Relaxed) + 1,
            dir,
            owner: Mutex::new(None),
            owner_known: Condvar::new(),
            stopping: AtomicBool::new(false),
            slots,
            limits: self.limits,
        });
        let endpoints = run
            .slots
            .iter()
            .map(|s| Endpoint {
                id: s.launch.id.clone(),
                socket: s.socket.clone(),
            })
            .collect();
        for (listener, slot) in listeners.into_iter().zip(run.slots.clone()) {
            let run = run.clone();
            thread::spawn(move || accept_loop(&run, &slot, &listener));
        }
        lock(&self.runs).insert(session, run);
        Ok(endpoints)
    }

    /// The agent of `session` runs as `pid`: only it and its descendants may
    /// connect, and the run ends when that process is gone.
    pub fn set_owner(&self, session: u32, pid: u32) {
        let Some(run) = lock(&self.runs).get(&session).cloned() else {
            return;
        };
        *lock(&run.owner) = Some(pid);
        run.owner_known.notify_all();
        let runs = Arc::downgrade(&self.runs);
        let token = run.token;
        drop(run);
        thread::spawn(move || {
            loop {
                thread::sleep(OWNER_POLL);
                let Some(runs) = runs.upgrade() else { return };
                let mut map = lock(&runs);
                if map.get(&session).is_none_or(|r| r.token != token) {
                    return;
                }
                if process_exists(pid) {
                    continue;
                }
                let run = map.remove(&session);
                drop(map);
                if let Some(run) = run {
                    run.stop();
                }
                return;
            }
        });
    }

    /// Stops the servers of `session`: SIGTERM to each process group, SIGKILL
    /// after the grace period, sockets removed.
    pub fn stop(&self, session: u32) {
        let run = lock(&self.runs).remove(&session);
        if let Some(run) = run {
            run.stop();
        }
    }

    /// Which run `session` has now, if any: to stop exactly that run later.
    pub fn run_token(&self, session: u32) -> Option<u64> {
        lock(&self.runs).get(&session).map(|r| r.token)
    }

    /// Stops the servers of `session` if its run is still `token`, and not a
    /// newer run that replaced it.
    pub fn stop_run(&self, session: u32, token: u64) {
        let run = {
            let mut runs = lock(&self.runs);
            match runs.get(&session) {
                Some(run) if run.token == token => runs.remove(&session),
                _ => None,
            }
        };
        if let Some(run) = run {
            run.stop();
        }
    }

    /// Stops every session's servers, as when the app quits.
    pub fn stop_all(&self) {
        let runs: Vec<Arc<Run>> = lock(&self.runs).drain().map(|(_, r)| r).collect();
        for run in runs {
            run.stop();
        }
    }

    /// What each server of `session` is doing, in attach order. Empty when the
    /// session has no run.
    pub fn states(&self, session: u32) -> Vec<(IntegrationId, McpServerState)> {
        lock(&self.runs).get(&session).map_or_else(Vec::new, |run| {
            run.slots
                .iter()
                .map(|s| (s.launch.id.clone(), lock(&s.inner).state.clone()))
                .collect()
        })
    }

    /// The process ids of servers running now, for every session.
    pub fn running_pids(&self) -> Vec<u32> {
        lock(&self.runs)
            .values()
            .flat_map(|run| run.slots.iter())
            .filter_map(|s| match lock(&s.inner).state {
                McpServerState::Running { pid } => Some(pid),
                _ => None,
            })
            .collect()
    }
}

impl Drop for McpRuntime {
    fn drop(&mut self) {
        self.stop_all();
    }
}

impl Run {
    fn stop(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        self.owner_known.notify_all();
        let groups = |slots: &[Arc<Slot>]| -> Vec<i32> {
            slots.iter().filter_map(|s| lock(&s.inner).group).collect()
        };
        for group in groups(&self.slots) {
            let _ = killpg(Pid::from_raw(group), Signal::SIGTERM);
        }
        let deadline = Instant::now() + self.limits.stop_grace;
        while !groups(&self.slots).is_empty() && Instant::now() < deadline {
            thread::sleep(POLL);
        }
        for group in groups(&self.slots) {
            let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while !groups(&self.slots).is_empty() && Instant::now() < deadline {
            thread::sleep(POLL);
        }
        for slot in &self.slots {
            if let Some(connection) = lock(&slot.inner).connection.take() {
                let _ = connection.shutdown(Shutdown::Both);
            }
        }
        remove_run_dir(&self.dir);
    }

    /// The agent's process id, waiting briefly for it right after the agent starts.
    fn owner(&self) -> Option<u32> {
        let deadline = Instant::now() + self.limits.owner_wait;
        let mut owner = lock(&self.owner);
        while owner.is_none() && !self.stopping.load(Ordering::SeqCst) {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            owner = self
                .owner_known
                .wait_timeout(owner, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
        *owner
    }
}

fn accept_loop(run: &Arc<Run>, slot: &Arc<Slot>, listener: &UnixListener) {
    while !run.stopping.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                let (run, slot) = (run.clone(), slot.clone());
                thread::spawn(move || serve(&run, &slot, stream));
            }
            Err(_) => thread::sleep(POLL),
        }
    }
}

/// One connection from the agent: checked, then a new server process for it.
fn serve(run: &Run, slot: &Slot, stream: UnixStream) {
    let set = |state: McpServerState| lock(&slot.inner).state = state;
    if stream.set_nonblocking(false).is_err() {
        return;
    }
    let peer = peer_pid(&stream);
    let owner = run.owner();
    let allowed = matches!((peer, owner), (Some(peer), Some(owner)) if is_descendant(peer, owner));
    if !allowed || run.stopping.load(Ordering::SeqCst) {
        // Not this session's agent: nothing starts, and the state stays as it was.
        return;
    }
    {
        let mut inner = lock(&slot.inner);
        if inner.group.is_some() {
            return;
        }
        inner.starts += 1;
        if inner.starts > run.limits.max_starts {
            inner.state = McpServerState::Failed {
                message: format!(
                    "the agent started it {} times in this run; it is not started again until the agent restarts",
                    run.limits.max_starts
                ),
            };
            return;
        }
    }
    let launch = &slot.launch;
    let child = Command::new(&launch.program)
        .args(&launch.args)
        .env_clear()
        .envs(launch.env.vars().iter().map(|(n, v)| (n, v)))
        .current_dir(&launch.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn();
    let mut child = match child {
        Ok(child) => child,
        Err(error) => {
            set(McpServerState::Failed {
                message: format!("could not start {}: {error}", launch.program.display()),
            });
            return;
        }
    };
    let pid = child.id();
    let group = i32::try_from(pid).unwrap_or(i32::MAX);
    {
        let mut inner = lock(&slot.inner);
        inner.state = McpServerState::Running { pid };
        inner.group = Some(group);
        inner.connection = stream.try_clone().ok();
    }
    let (Some(mut stdin), Some(mut stdout), Some(mut stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
        return;
    };
    let first_request = Arc::new(Mutex::new(None::<Instant>));
    let answered = Arc::new(AtomicBool::new(false));

    // Agent → server.
    if let Ok(mut from_agent) = stream.try_clone() {
        let first_request = first_request.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match from_agent.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        lock(&first_request).get_or_insert_with(Instant::now);
                        if stdin
                            .write_all(&buf[..n])
                            .and_then(|()| stdin.flush())
                            .is_err()
                        {
                            break;
                        }
                    }
                }
            }
            // Closing stdin tells an MCP server to exit.
        });
    }
    // Server → agent.
    if let Ok(mut to_agent) = stream.try_clone() {
        let answered = answered.clone();
        thread::spawn(move || {
            let mut buf = [0u8; 16 * 1024];
            loop {
                match stdout.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        answered.store(true, Ordering::SeqCst);
                        if to_agent.write_all(&buf[..n]).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = to_agent.shutdown(Shutdown::Both);
        });
    }
    // Error output: drained so the server never blocks on it, the tail kept.
    let tail = Arc::new(Mutex::new(Vec::<u8>::new()));
    {
        let tail = tail.clone();
        let cap = run.limits.stderr_bytes;
        thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while let Ok(n) = stderr.read(&mut buf) {
                if n == 0 {
                    break;
                }
                let mut tail = lock(&tail);
                tail.extend_from_slice(&buf[..n]);
                if tail.len() > cap {
                    let excess = tail.len() - cap;
                    tail.drain(..excess);
                }
            }
        });
    }

    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => {}
            Err(_) => break None,
        }
        let waiting =
            lock(&first_request).is_some_and(|at| at.elapsed() > run.limits.startup_timeout);
        if waiting && !answered.load(Ordering::SeqCst) && !timed_out {
            timed_out = true;
            let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
        }
        thread::sleep(POLL);
    };
    // Whatever it started goes with it.
    let _ = killpg(Pid::from_raw(group), Signal::SIGKILL);
    let _ = stream.shutdown(Shutdown::Both);

    let stopping = run.stopping.load(Ordering::SeqCst);
    let last_line = || {
        let tail = String::from_utf8_lossy(&lock(&tail)).into_owned();
        let line = tail
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("")
            .trim()
            .to_owned();
        let line: String = launch.env.redact(&line).chars().take(300).collect();
        if line.is_empty() {
            String::new()
        } else {
            format!(": {line}")
        }
    };
    let state = if timed_out {
        McpServerState::Failed {
            message: format!(
                "did not answer within {} s of the agent's first request; stopped",
                run.limits.startup_timeout.as_secs_f32()
            ),
        }
    } else {
        match status {
            Some(status) if status.success() || stopping => McpServerState::Exited {
                code: status.code(),
            },
            Some(status) => {
                use std::os::unix::process::ExitStatusExt;
                let how = match (status.code(), status.signal()) {
                    (Some(code), _) => format!("exited with code {code}"),
                    (None, Some(signal)) => format!("was killed by signal {signal}"),
                    _ => "ended".to_owned(),
                };
                McpServerState::Failed {
                    message: format!("{how}{}", last_line()),
                }
            }
            None => McpServerState::Failed {
                message: "could not be watched".to_owned(),
            },
        }
    };
    let mut inner = lock(&slot.inner);
    inner.state = state;
    inner.group = None;
    inner.connection = None;
}

#[cfg(target_os = "macos")]
fn peer_pid(stream: &UnixStream) -> Option<u32> {
    use nix::sys::socket::{getsockopt, sockopt::LocalPeerPid};
    getsockopt(stream, LocalPeerPid)
        .ok()
        .and_then(|pid| u32::try_from(pid).ok())
}

#[cfg(target_os = "linux")]
fn peer_pid(stream: &UnixStream) -> Option<u32> {
    use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
    getsockopt(stream, PeerCredentials)
        .ok()
        .and_then(|c| u32::try_from(c.pid()).ok())
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn peer_pid(_: &UnixStream) -> Option<u32> {
    None
}

/// Whether a process `pid` exists (one that is not ours to signal counts).
fn process_exists(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    !matches!(
        nix::sys::signal::kill(Pid::from_raw(pid), None),
        Err(nix::errno::Errno::ESRCH)
    )
}

/// Whether `pid` is `ancestor` or one of its descendants.
fn is_descendant(mut pid: u32, ancestor: u32) -> bool {
    for _ in 0..64 {
        if pid == ancestor {
            return true;
        }
        if pid <= 1 {
            return false;
        }
        match parent_of(pid) {
            Some(parent) => pid = parent,
            None => return false,
        }
    }
    false
}

fn parent_of(pid: u32) -> Option<u32> {
    let ps = ["/bin/ps", "/usr/bin/ps"]
        .into_iter()
        .find(|p| Path::new(p).is_file())?;
    let output = Command::new(ps)
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8(output.stdout).ok()?.trim().parse().ok()
}

fn private_dir(dir: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_symlink() => Err(std::io::Error::other("is a symbolic link")),
        Ok(_) => std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(dir)?;
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
        }
        Err(e) => Err(e),
    }
}

/// `<session>-<token>`: the only directories this module makes or removes.
fn is_run_dir_name(name: &str) -> bool {
    name.split_once('-').is_some_and(|(session, token)| {
        !session.is_empty()
            && session.bytes().all(|b| b.is_ascii_digit())
            && token.len() == 8
            && token.bytes().all(|b| b.is_ascii_hexdigit())
    })
}

/// Removes a run directory: its socket files, then the directory. Nothing else is
/// ever in it; anything else stops the removal.
fn remove_run_dir(dir: &Path) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "sock") {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let _ = std::fs::remove_dir(dir);
}

fn token() -> String {
    let mut bytes = [0u8; 4];
    if let Ok(mut random) = std::fs::File::open("/dev/urandom") {
        let _ = random.read_exact(&mut bytes);
    }
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
