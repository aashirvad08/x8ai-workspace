//! An app started from a terminal (`x8ai`, or `pnpm tauri dev`): its sessions
//! inherit its environment, except the variables that point at that terminal.
//! The only test in its binary, since it changes the process environment.

use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_pty::{Environment, Program, SessionEvents, Sessions};

#[derive(Default)]
struct Recorder {
    output: Mutex<Vec<u8>>,
    changed: Condvar,
}

impl SessionEvents for Recorder {
    fn output(&self, bytes: Vec<u8>) {
        self.output.lock().unwrap().extend(bytes);
        self.changed.notify_all();
    }
    fn error(&self, _: String) {}
    fn exited(&self, _: TerminalExit) {
        self.changed.notify_all();
    }
}

impl Recorder {
    fn wait_for(&self, needle: &str) -> String {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut output = self.output.lock().unwrap();
        loop {
            let text = String::from_utf8_lossy(&output).into_owned();
            if text.contains(needle) {
                return text;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "no {needle:?} in {text:?}");
            output = self.changed.wait_timeout(output, left).unwrap().0;
        }
    }
}

#[test]
fn a_session_is_not_told_it_runs_inside_the_apps_own_terminal() {
    // SAFETY: this binary's only test, and nothing else runs yet: no other thread
    // reads the environment while it changes.
    unsafe {
        std::env::set_var("TMUX", "/tmp/tmux-501/default,1234,0");
        std::env::set_var("KITTY_WINDOW_ID", "3");
        std::env::set_var("X8AI_INHERITED_TEST", "kept");
    }
    let sessions = Sessions::default();
    let size = TerminalSize {
        cols: 100,
        rows: 30,
    };
    for program in [
        Program::LoginShell {
            cwd: None,
            env: Vec::new(),
        },
        Program::Exec {
            program: "/bin/sh".into(),
            args: Vec::new(),
            cwd: None,
            env: Environment::Inherit,
        },
    ] {
        let recorder = Arc::new(Recorder::default());
        let session = sessions.spawn(&program, size, recorder.clone()).unwrap();
        // Computed output, so the echoed input cannot match.
        session
            .write(
                b"printf 'tmux=%s kitty=%s other=%s sum=%s\\n' \"${TMUX-absent}\" \"${KITTY_WINDOW_ID-absent}\" \"$X8AI_INHERITED_TEST\" \"$((40 + 2))\"\n"
                    .to_vec(),
            )
            .unwrap();
        let output = recorder.wait_for("sum=42");
        assert!(
            output.contains("tmux=absent kitty=absent other=kept sum=42"),
            "{program:?}: {output}"
        );
        sessions.close(session.id()).unwrap();
    }
    sessions.shutdown(Duration::from_secs(1));
}
