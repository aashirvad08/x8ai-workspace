//! `x8ai` end to end: the real binary on a PTY, its screen read back through
//! the same emulator the panes use, driven by typed keys. A home folder and a
//! data folder of its own keep it away from the user's.

use std::cell::RefCell;
use std::path::Path;
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

const TIMEOUT: Duration = Duration::from_secs(15);
const SIZE: TerminalSize = TerminalSize {
    cols: 100,
    rows: 30,
};
const CTRL_G: &str = "\x07";

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
struct X8ai {
    _sessions: Sessions,
    session: Arc<Session>,
    events: Receiver<Event>,
    term: Term<Listener>,
    parser: Processor,
    listener: Listener,
    exit: Option<TerminalExit>,
}

impl X8ai {
    fn start(home: &Path, args: &[&str]) -> Self {
        let (tx, events) = mpsc::channel();
        let sessions = Sessions::default();
        let program = Program::Exec {
            program: env!("CARGO_BIN_EXE_x8ai").into(),
            args: args.iter().map(Into::into).collect(),
            cwd: Some(home.to_owned()),
            env: Environment::Exactly(vec![
                ("HOME".into(), home.display().to_string()),
                ("PATH".into(), "/usr/bin:/bin:/usr/sbin:/sbin".into()),
                ("SHELL".into(), "/bin/sh".into()),
                ("LANG".into(), "en_US.UTF-8".into()),
                (
                    "X8AI_DATA_DIR".into(),
                    home.join(".x8ai-data").display().to_string(),
                ),
            ]),
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
        }
    }

    fn keys(&self, text: &str) {
        self.session.write(text.as_bytes().to_vec()).unwrap();
    }

    /// Handles one event from `x8ai`, waiting at most until `deadline`.
    fn pump(&mut self, deadline: Instant) -> bool {
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

    fn screen(&self) -> String {
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

    /// Waits until the screen shows `needle`; returns the screen.
    fn wait_for(&mut self, needle: &str) -> String {
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

    fn wait_for_exit(&mut self) -> TerminalExit {
        let deadline = Instant::now() + TIMEOUT;
        while self.exit.is_none() {
            assert!(self.pump(deadline), "x8ai did not quit:\n{}", self.screen());
        }
        self.exit.clone().unwrap()
    }
}

#[test]
fn welcome_new_space_shell_and_back() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    let mut x8ai = X8ai::start(&home, &[]);

    let screen = x8ai.wait_for("W E L C O M E ,");
    assert!(screen.contains("S I R"), "{screen}");
    // Nothing opened yet: Esc goes to the workspace with no folder.
    assert!(screen.contains("Home no folder open"), "{screen}");

    // `/new` makes the space and opens its shell, in its folder, with its id.
    x8ai.keys("/new demo\r");
    x8ai.wait_for(" x8ai demo  ~/Workspaces/demo  ws-");
    x8ai.keys("echo \"in:$(pwd):$X8AI_SPACE:$((40 + 2))\"\r");
    let screen = x8ai.wait_for(":42");
    let demo = home.join("Workspaces/demo");
    assert!(
        screen.contains(&format!("in:{}:ws-", demo.display())),
        "{screen}"
    );

    // Ctrl-g h: the Welcome over the space, which keeps running.
    x8ai.keys(&format!("{CTRL_G}h"));
    let screen = x8ai.wait_for("Space demo");
    assert!(screen.contains("~/Workspaces/demo · ws-"), "{screen}");
    assert!(screen.contains("not trusted"), "{screen}");

    // `/home`: the workspace with no folder, a shell in the home folder.
    x8ai.keys("/home\r");
    x8ai.wait_for(" x8ai Home  ~");
    x8ai.keys("echo \"home:$(pwd):$((6 * 7))\"\r");
    x8ai.wait_for(&format!("home:{}:42", home.display()));

    // Back to demo by its name: its shell is as it was left.
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("Home no folder open");
    x8ai.keys("/cd dem\r");
    let screen = x8ai.wait_for(" x8ai demo ");
    assert!(screen.contains("in:"), "{screen}");

    // A folder that is not there is explained, not opened.
    x8ai.keys(&format!("{CTRL_G}h"));
    x8ai.wait_for("Space demo");
    x8ai.keys("/cd ~/nowhere\r");
    x8ai.wait_for("There is no folder ~/nowhere.");

    // Quitting with a program running asks first.
    x8ai.keys("\x1b");
    x8ai.wait_for(" x8ai demo ");
    x8ai.keys("sleep 60\r");
    // Give the shell a moment to start the job in the foreground.
    std::thread::sleep(Duration::from_millis(500));
    x8ai.keys(&format!("{CTRL_G}q"));
    let screen = x8ai.wait_for("Quit x8ai?");
    assert!(screen.contains("still running in demo"), "{screen}");
    x8ai.keys("y");
    let exit = x8ai.wait_for_exit();
    assert_eq!(exit.code, 0, "{exit:?}");

    // The space is remembered where the app keeps its recent spaces.
    let recent = std::fs::read_to_string(home.join(".x8ai-data/recent-workspaces.json")).unwrap();
    assert!(recent.contains(&demo.display().to_string()), "{recent}");
}

#[test]
fn a_folder_given_on_the_command_line_opens_at_once() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().canonicalize().unwrap();
    std::fs::create_dir(home.join("app")).unwrap();
    let mut x8ai = X8ai::start(&home, &["app"]);
    x8ai.wait_for(" x8ai app  ~/app");
    x8ai.keys("echo \"at:$(pwd)\"\r");
    x8ai.wait_for(&format!("at:{}", home.join("app").display()));
    // The shell ends: Enter starts another.
    x8ai.keys("exit\r");
    x8ai.wait_for("The shell exited with 0.");
    x8ai.keys("\r");
    x8ai.keys("echo \"again:$((1 + 1))\"\r");
    x8ai.wait_for("again:2");
    x8ai.keys(&format!("{CTRL_G}q"));
    assert_eq!(x8ai.wait_for_exit().code, 0);
}
