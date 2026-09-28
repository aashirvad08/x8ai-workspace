//! End-to-end tests against real PTYs and real processes. `/bin/sh` is used for
//! determinism; one test exercises the user's actual login shell.

use std::path::PathBuf;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use nix::sys::signal::kill;
use nix::unistd::Pid;
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_pty::{
    ACK_BYTES, Error, FLOW_WINDOW, KILL_GRACE, Program, Session, SessionEvents, Sessions,
};

const SIZE: TerminalSize = TerminalSize { cols: 80, rows: 24 };
const TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq)]
enum Event {
    Output(usize),
    Error(String),
    Exited(TerminalExit),
}

/// Records everything a session delivers.
#[derive(Default)]
struct Recorder {
    state: Mutex<Recorded>,
    changed: Condvar,
}

#[derive(Default)]
struct Recorded {
    output: Vec<u8>,
    events: Vec<Event>,
}

impl SessionEvents for Recorder {
    fn output(&self, bytes: Vec<u8>) {
        let mut state = self.state.lock().unwrap();
        state.events.push(Event::Output(bytes.len()));
        state.output.extend(bytes);
        self.changed.notify_all();
    }
    fn error(&self, message: String) {
        self.state
            .lock()
            .unwrap()
            .events
            .push(Event::Error(message));
        self.changed.notify_all();
    }
    fn exited(&self, exit: TerminalExit) {
        self.state.lock().unwrap().events.push(Event::Exited(exit));
        self.changed.notify_all();
    }
}

impl Recorder {
    fn wait_until(&self, what: &str, done: impl Fn(&Recorded) -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        let mut state = self.state.lock().unwrap();
        while !done(&state) {
            let now = Instant::now();
            assert!(
                now < deadline,
                "timed out waiting for {what}; output so far:\n{}",
                String::from_utf8_lossy(&state.output)
            );
            state = self.changed.wait_timeout(state, deadline - now).unwrap().0;
        }
    }

    fn wait_for_output(&self, needle: &str) {
        self.wait_until(&format!("{needle:?}"), |r| {
            String::from_utf8_lossy(&r.output).contains(needle)
        });
    }

    fn wait_for_exit(&self) -> TerminalExit {
        self.wait_until("exit", |r| {
            matches!(r.events.last(), Some(Event::Exited(_)))
        });
        match self.state.lock().unwrap().events.last() {
            Some(Event::Exited(exit)) => exit.clone(),
            _ => unreachable!(),
        }
    }

    fn output(&self) -> String {
        String::from_utf8_lossy(&self.state.lock().unwrap().output).into_owned()
    }
}

fn exec(program: &str, args: &[&str]) -> Program {
    Program::Exec {
        program: PathBuf::from(program),
        args: args.iter().map(Into::into).collect(),
        cwd: None,
    }
}

fn sh() -> Program {
    exec("/bin/sh", &[])
}

fn start(program: &Program) -> (Arc<Session>, Arc<Recorder>) {
    let recorder = Arc::new(Recorder::default());
    let session = Sessions::default()
        .spawn(program, SIZE, recorder.clone())
        .expect("session should start");
    (session, recorder)
}

fn type_line(session: &Session, line: &str) {
    session.write(format!("{line}\n").into_bytes()).unwrap();
}

fn is_alive(pid: u32) -> bool {
    kill(Pid::from_raw(pid as i32), None).is_ok()
}

#[test]
fn streams_output_and_reports_exit_after_it() {
    let (_session, recorder) = start(&exec("/bin/sh", &["-c", "printf 'hello from pty\\n'"]));

    let exit = recorder.wait_for_exit();
    assert_eq!(
        exit,
        TerminalExit {
            code: 0,
            signal: None
        }
    );
    assert!(recorder.output().contains("hello from pty"));
    let events = recorder.state.lock().unwrap().events.clone();
    assert!(matches!(events.first(), Some(Event::Output(_))));
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, Event::Exited(_)))
            .count(),
        1
    );
}

#[test]
fn short_lived_output_survives_concurrent_spawns() {
    // Regression test: concurrent `openpty` calls fail intermittently on macOS
    // unless serialized.
    let sessions = Arc::new(Sessions::default());
    let workers: Vec<_> = (0..48)
        .map(|i| {
            let sessions = sessions.clone();
            std::thread::spawn(move || {
                let recorder = Arc::new(Recorder::default());
                let program = exec("/bin/echo", &[&format!("marker-{i}")]);
                let _session = sessions.spawn(&program, SIZE, recorder.clone()).unwrap();
                recorder.wait_for_exit();
                recorder.output()
            })
        })
        .collect();
    for (i, worker) in workers.into_iter().enumerate() {
        let output = worker.join().unwrap();
        assert!(
            output.contains(&format!("marker-{i}")),
            "session {i} lost its output: {output:?}"
        );
    }
}

#[test]
fn reports_the_exit_code() {
    let (_session, recorder) = start(&exec("/bin/sh", &["-c", "exit 3"]));
    assert_eq!(recorder.wait_for_exit().code, 3);
}

#[test]
fn runs_an_interactive_shell() {
    let (session, recorder) = start(&sh());
    // The expected text is computed by the shell, so the echoed input cannot match.
    type_line(&session, "echo result-$((6 * 7))");
    recorder.wait_for_output("result-42");
}

#[test]
fn provides_a_terminal_environment() {
    let (session, recorder) = start(&sh());
    type_line(
        &session,
        "printf '%s|%s|' \"$TERM\" \"$COLORTERM\"; test -t 0 && echo is-a-tty",
    );
    recorder.wait_for_output("xterm-256color|truecolor|is-a-tty");
}

#[test]
fn starts_in_the_home_directory() {
    let home = std::env::var("HOME").expect("HOME is set");
    let (_session, recorder) = start(&exec("/bin/pwd", &[]));
    recorder.wait_for_exit();
    assert_eq!(recorder.output().trim(), home);
}

#[test]
fn ctrl_c_interrupts_the_foreground_process() {
    let (session, recorder) = start(&sh());
    type_line(&session, "sleep 30");
    std::thread::sleep(Duration::from_millis(300));
    let interrupted_at = Instant::now();
    session.write(vec![0x03]).unwrap();
    type_line(&session, "echo after-$((1 + 1))");

    // The shell only runs the next command once sleep has died.
    recorder.wait_for_output("after-2");
    assert!(interrupted_at.elapsed() < Duration::from_secs(5));
}

#[test]
fn ctrl_d_ends_the_shell() {
    let (session, recorder) = start(&sh());
    session.write(vec![0x04]).unwrap();
    assert_eq!(recorder.wait_for_exit().code, 0);
    assert!(matches!(
        session.write(b"echo hi\n".to_vec()),
        Err(Error::Exited)
    ));
}

#[test]
fn ctrl_z_suspends_the_foreground_job() {
    let (session, recorder) = start(&sh());
    type_line(&session, "sleep 30");
    std::thread::sleep(Duration::from_millis(300));
    session.write(vec![0x1a]).unwrap();
    type_line(&session, "jobs");
    recorder.wait_until("a stopped job", |r| {
        let out = String::from_utf8_lossy(&r.output).to_lowercase();
        out.contains("stopped") || out.contains("suspended")
    });
}

#[test]
fn resize_reaches_the_program() {
    let (session, recorder) = start(&sh());
    type_line(&session, "stty size");
    recorder.wait_for_output("24 80");

    session
        .resize(TerminalSize {
            cols: 132,
            rows: 43,
        })
        .unwrap();
    type_line(&session, "stty size");
    recorder.wait_for_output("43 132");
}

#[test]
fn rejects_out_of_range_sizes() {
    let (session, _recorder) = start(&sh());
    let err = session
        .resize(TerminalSize { cols: 0, rows: 24 })
        .unwrap_err();
    assert!(matches!(err, Error::InvalidSize(_)));

    let recorder = Arc::new(Recorder::default());
    let spawned = Sessions::default().spawn(&sh(), TerminalSize { cols: 80, rows: 0 }, recorder);
    assert!(matches!(spawned, Err(Error::InvalidSize(_))));
}

#[test]
fn reports_programs_that_cannot_start() {
    let recorder = Arc::new(Recorder::default());
    let spawned = Sessions::default().spawn(&exec("/nonexistent/program", &[]), SIZE, recorder);
    assert!(matches!(spawned, Err(Error::Spawn(_))));
}

#[test]
fn output_pauses_until_acknowledged() {
    const TOTAL: usize = 3 * FLOW_WINDOW;
    let (session, recorder) = start(&exec(
        "/bin/sh",
        &["-c", &format!("head -c {TOTAL} /dev/zero")],
    ));

    // Without acknowledgements delivery stops at the flow window.
    recorder.wait_until("the flow window to fill", |r| {
        r.output.len() >= FLOW_WINDOW - ACK_BYTES as usize
    });
    std::thread::sleep(Duration::from_millis(300));
    let delivered = recorder.state.lock().unwrap().output.len();
    assert!(
        delivered <= FLOW_WINDOW,
        "delivered {delivered} bytes without acknowledgement"
    );
    assert!(
        !session.has_exited(),
        "the producer should be blocked, not finished"
    );

    // Acknowledging lets the rest through.
    let mut acked = 0;
    let deadline = Instant::now() + TIMEOUT;
    while !matches!(
        recorder.state.lock().unwrap().events.last(),
        Some(Event::Exited(_))
    ) {
        assert!(
            Instant::now() < deadline,
            "stalled after acknowledging {acked} bytes"
        );
        let received = recorder.state.lock().unwrap().output.len();
        session.ack((received - acked) as u32);
        acked = received;
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(recorder.state.lock().unwrap().output.len(), TOTAL);
}

#[test]
fn close_hangs_up_the_process() {
    let (session, _recorder) = start(&sh());
    let pid = session.pid().unwrap();
    assert!(is_alive(pid));

    session.close();
    assert!(session.wait_for_exit(Duration::from_secs(3)));
    assert!(!is_alive(pid));
}

#[test]
fn close_kills_a_process_that_ignores_hangup() {
    let (session, _recorder) = start(&exec("/bin/sh", &["-c", "trap '' HUP; sleep 30"]));
    std::thread::sleep(Duration::from_millis(200));

    session.close();
    assert!(
        !session.wait_for_exit(KILL_GRACE / 2),
        "SIGHUP should have been ignored"
    );
    assert!(
        session.wait_for_exit(KILL_GRACE * 2),
        "SIGKILL should follow the grace period"
    );
}

#[test]
fn the_registry_tracks_and_closes_sessions() {
    let sessions = Sessions::default();
    let a = sessions
        .spawn(&sh(), SIZE, Arc::new(Recorder::default()))
        .unwrap();
    let b = sessions
        .spawn(&sh(), SIZE, Arc::new(Recorder::default()))
        .unwrap();
    assert_ne!(a.id(), b.id());
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions.get(a.id()).unwrap().pid(), a.pid());

    sessions.close(a.id()).unwrap();
    assert!(matches!(sessions.get(a.id()), Err(Error::NotFound(_))));
    assert!(matches!(sessions.close(a.id()), Err(Error::NotFound(_))));
    assert!(a.wait_for_exit(Duration::from_secs(3)));

    sessions.shutdown(Duration::from_secs(1));
    assert!(sessions.is_empty());
    assert!(b.has_exited());
}

#[test]
fn the_login_shell_is_the_users_shell() {
    let (session, recorder) = start(&Program::LoginShell);
    assert!(session.program().starts_with('/'), "{}", session.program());
    // argv[0] of a login shell starts with '-'. Computed output avoids matching the
    // echoed input.
    type_line(
        &session,
        "printf 'argv0=%s pwd=%s sum=%s\\n' \"$0\" \"$PWD\" \"$((20 + 22))\"",
    );
    recorder.wait_for_output("sum=42");

    let home = std::env::var("HOME").unwrap();
    let out = recorder.output();
    assert!(out.contains("argv0=-"), "not a login shell: {out}");
    assert!(
        out.contains(&format!("pwd={home}")),
        "not started in {home}: {out}"
    );
}
