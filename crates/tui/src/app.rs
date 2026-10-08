//! The app: the Welcome screen and the open space, what each key does there,
//! and the loop that reads keys and programs' output and redraws.
//!
//! One space is shown at a time, as in the desktop app. Each space opened in
//! this run keeps its shell while `x8ai` runs, so going back to it with `/cd`
//! finds the shell as it was left.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::CursorShape;
use ratatui::crossterm::cursor::SetCursorStyle;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::crossterm::execute;
use ratatui::{DefaultTerminal, Frame};
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_core::workspace::RecentWorkspace;
use x8ai_pty::{Program, Sessions};

use crate::keys;
use crate::pane::{Pane, PaneId};
use crate::spaces::{SpaceInfo, Spaces};
use crate::theme::Theme;
use crate::ui;
use crate::welcome::{self, Command, Context, Line, MAX_NAME_LENGTH, Suggestion};

/// How long programs get to exit after hangup when `x8ai` quits.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// The longest the loop handles messages before drawing, so a flood of output
/// still shows as it arrives.
const FRAME_BUDGET: Duration = Duration::from_millis(16);

/// Rows the space's own lines take: its header and the status bar.
const CHROME_ROWS: u16 = 2;

/// What reaches the loop: keys from the user's terminal, and programs' output.
pub enum Msg {
    Input(Event),
    /// Reading the user's terminal failed; nothing more can be typed.
    InputClosed(String),
    Output(PaneId, Vec<u8>),
    PaneError(PaneId, String),
    Exited(PaneId, TerminalExit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Space,
}

/// What keys do in a space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Keys go to the shell; Ctrl-g is the prefix for x8ai's own keys.
    Normal,
    /// Ctrl-g was pressed: the next key is x8ai's.
    Prefix,
    /// Reading the scrollback: keys move the view.
    Scroll,
}

/// Ctrl-g: the prefix for x8ai's keys in a space. Not Ctrl-Space, which macOS
/// keeps for switching input sources, nor Ctrl-b (tmux) or Ctrl-a (readline).
const PREFIX: char = 'g';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub error: bool,
}

/// A program still running when the user asked to quit.
pub struct QuitQuestion {
    pub busy: Vec<String>,
}

/// A space opened in this run, and its shell.
struct SpacePane {
    space: SpaceInfo,
    pane: Pane,
}

pub struct App {
    pub theme: Theme,
    pub screen: Screen,
    pub mode: Mode,
    pub line: Line,
    /// The suggestion chosen with ↑↓, if any.
    pub selected: Option<usize>,
    pub suggestions: Vec<Suggestion>,
    pub message: Option<Message>,
    recent: Vec<RecentWorkspace>,
    chosen_name: Option<String>,
    account_name: Option<String>,
    /// The open space; the Welcome screen shows over it.
    pub current: SpaceInfo,
    panes: Vec<SpacePane>,
    pub quit_question: Option<QuitQuestion>,
    /// Why `x8ai` stopped, when it was not the user's choice.
    pub failure: Option<String>,
    spaces: Spaces,
    sessions: Sessions,
    tx: Sender<Msg>,
    cwd: PathBuf,
    next_pane: u32,
    size: (u16, u16),
    cursor: Option<(CursorShape, bool)>,
    quit: bool,
}

impl App {
    pub fn new(mut spaces: Spaces, cwd: PathBuf, tx: Sender<Msg>, size: (u16, u16)) -> Self {
        // The last space opens behind the Welcome, as in the app: Esc goes to it.
        let recent = spaces.recent();
        let current = recent
            .first()
            .filter(|r| r.available)
            .and_then(|r| spaces.reopen(Path::new(&r.root)).ok())
            .unwrap_or_else(|| spaces.home_space());
        let mut app = Self {
            theme: Theme::detect(),
            screen: Screen::Welcome,
            mode: Mode::Normal,
            line: Line::default(),
            selected: None,
            suggestions: Vec::new(),
            message: None,
            recent: Vec::new(),
            chosen_name: spaces.chosen_name(),
            account_name: crate::spaces::account_name(),
            current,
            panes: Vec::new(),
            quit_question: None,
            failure: None,
            spaces,
            sessions: Sessions::default(),
            tx,
            cwd,
            next_pane: 0,
            size,
            cursor: None,
            quit: false,
        };
        app.refresh_recent();
        app.show_warnings();
        app
    }

    /// Who the welcome greets.
    pub fn name(&self) -> Option<&str> {
        self.chosen_name.as_deref().or(self.account_name.as_deref())
    }

    pub fn home(&self) -> &Path {
        self.spaces.home()
    }

    /// The open space's pane, if its shell has started.
    pub fn current_pane(&self) -> Option<&Pane> {
        self.panes
            .iter()
            .find(|p| p.space.root == self.current.root)
            .map(|p| &p.pane)
    }

    fn current_pane_mut(&mut self) -> Option<&mut Pane> {
        let root = self.current.root.clone();
        self.panes
            .iter_mut()
            .find(|p| p.space.root == root)
            .map(|p| &mut p.pane)
    }

    /// The size of a space's pane: the window, less the header and status bar.
    fn pane_size(&self) -> TerminalSize {
        TerminalSize {
            cols: self.size.0.max(1),
            rows: self.size.1.saturating_sub(CHROME_ROWS).max(1),
        }
    }

    // Welcome

    pub fn say(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            error: false,
        });
    }

    pub fn complain(&mut self, text: impl Into<String>) {
        self.message = Some(Message {
            text: text.into(),
            error: true,
        });
    }

    fn show_warnings(&mut self) {
        let warnings = self.spaces.take_warnings();
        if !warnings.is_empty() {
            self.complain(warnings.join(" "));
        }
    }

    fn refresh_recent(&mut self) {
        self.recent = self.spaces.recent();
        self.show_warnings();
        self.refresh_suggestions();
    }

    fn refresh_suggestions(&mut self) {
        let context = Context {
            recent: &self.recent,
            open: self.current.root.as_deref(),
            cwd: &self.cwd,
            home: self.spaces.home(),
        };
        self.suggestions = welcome::suggestions(self.line.text(), &context);
        self.selected = None;
    }

    /// The Welcome screen over the space, which keeps running.
    pub fn show_welcome(&mut self) {
        self.screen = Screen::Welcome;
        self.mode = Mode::Normal;
        self.message = None;
        self.refresh_recent();
    }

    /// The open space, its shell started if it has none yet.
    fn show_space(&mut self) {
        let size = self.pane_size();
        if self.current_pane().is_none() {
            let id = PaneId(self.next_pane);
            self.next_pane += 1;
            // The space's id, as the app's terminals have it (ADR 0019).
            let env = self
                .current
                .id
                .iter()
                .map(|id| ("X8AI_SPACE".to_owned(), id.clone()))
                .collect();
            let program = Program::LoginShell {
                cwd: self.current.root.clone(),
                env,
            };
            match Pane::start(id, &self.sessions, &program, size, self.tx.clone()) {
                Ok(pane) => self.panes.push(SpacePane {
                    space: self.current.clone(),
                    pane,
                }),
                Err(error) => {
                    self.screen = Screen::Welcome;
                    self.complain(format!("Could not start a shell: {error}"));
                    return;
                }
            }
        }
        self.screen = Screen::Space;
        self.mode = Mode::Normal;
        self.line.take();
    }

    /// Forgets the open space's pane, whose shell has ended, and starts another.
    fn restart_shell(&mut self) {
        let root = self.current.root.clone();
        if let Some(at) = self.panes.iter().position(|p| p.space.root == root) {
            let ended = self.panes.remove(at);
            let _ = self.sessions.close(ended.pane.session_id());
        }
        self.show_space();
    }

    /// Opens `space` and goes to it.
    fn go_to(&mut self, space: SpaceInfo) {
        self.current = space;
        self.show_warnings();
        self.show_space();
    }

    fn run_line(&mut self) {
        // A suggestion chosen with ↑↓ is taken whole: Enter on a recent space
        // opens it.
        if let Some(suggestion) = self.selected.and_then(|i| self.suggestions.get(i)) {
            let completion = suggestion.completion.clone();
            self.line.set(&completion);
            if completion.ends_with(' ') {
                // A command that needs an argument: keep typing.
                self.refresh_suggestions();
                return;
            }
        }
        let text = self.line.take();
        self.message = None;
        match welcome::parse(&text) {
            Command::Empty => self.show_space(),
            Command::Cd(arg) => self.cd(arg),
            Command::New("") => {
                self.complain("Name the new space: /new <name>. It is made in ~/Workspaces.")
            }
            Command::New(name) => match self.spaces.create(name) {
                Ok(space) => self.go_to(space),
                Err(error) => self.complain(error),
            },
            Command::Home => {
                let home = self.spaces.home_space();
                self.go_to(home);
            }
            Command::Name(name) => {
                let name: String = name.chars().take(MAX_NAME_LENGTH).collect();
                let chosen = (!name.is_empty()).then_some(name.as_str());
                match self.spaces.choose_name(chosen) {
                    Ok(()) => {
                        self.chosen_name = chosen.map(str::to_owned);
                        match chosen {
                            Some(name) => self.say(format!("Hello, {name}.")),
                            None => self.say("Greeting you with your account's name again."),
                        }
                    }
                    Err(error) => self.complain(error),
                }
            }
            Command::Quit => self.ask_to_quit(),
            Command::AppOnly(word) => {
                self.complain(format!(
                    "{word} is not in x8ai's terminal version yet. Use the app for it for now."
                ));
            }
            Command::Unknown(word) => self.complain(format!(
                "“{word}” is not a command here. Try /cd <folder>, /new <name>, /home or /quit."
            )),
        }
        self.refresh_suggestions();
    }

    /// `/cd <arg>`: a recent space by its path or name, else a folder.
    fn cd(&mut self, arg: &str) {
        if arg.is_empty() {
            self.say("Type a folder after /cd, or choose a recent space with ↑↓ and press Enter.");
            return;
        }
        let matches = welcome::match_recent(arg, &self.recent);
        if matches.len() > 1 {
            let roots: Vec<String> = matches
                .iter()
                .map(|m| welcome::tilde(Path::new(&m.root), self.spaces.home()))
                .collect();
            self.complain(format!(
                "Several spaces match “{arg}”: {}. Type more of the path.",
                roots.join(", ")
            ));
            return;
        }
        if let Some(found) = matches.first() {
            let root = PathBuf::from(&found.root);
            if self.current.root.as_deref() == Some(root.as_path()) {
                self.show_space();
                return;
            }
            match self.spaces.reopen(&root) {
                Ok(space) => self.go_to(space),
                Err(error) => {
                    self.complain(error);
                    self.refresh_recent();
                }
            }
            return;
        }
        let path = welcome::resolve(arg, &self.cwd, self.spaces.home());
        if !path.is_dir() {
            let shown = welcome::tilde(&path, self.spaces.home());
            self.complain(format!(
                "There is no folder {shown}. /new <name> makes a new space in ~/Workspaces."
            ));
            return;
        }
        match self.spaces.open(&path) {
            Ok(space) => self.go_to(space),
            Err(error) => self.complain(format!("Could not open {arg}: {error}")),
        }
    }

    /// Opens `folder` (`x8ai <folder>`) and goes to it; on failure the Welcome
    /// says why.
    pub fn open_at_start(&mut self, folder: &str) {
        let path = welcome::resolve(folder, &self.cwd, self.spaces.home());
        if !path.is_dir() {
            let shown = welcome::tilde(&path, self.spaces.home());
            self.complain(format!("There is no folder {shown}."));
            return;
        }
        match self.spaces.open(&path) {
            Ok(space) => self.go_to(space),
            Err(error) => self.complain(format!("Could not open {folder}: {error}")),
        }
    }

    fn welcome_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Enter => self.run_line(),
            KeyCode::Esc => {
                if self.line.is_empty() {
                    self.message = None;
                    self.show_space();
                } else {
                    self.line.take();
                    self.refresh_suggestions();
                }
            }
            KeyCode::Char('c') if ctrl => {
                if self.line.is_empty() {
                    self.ask_to_quit();
                } else {
                    self.line.take();
                    self.refresh_suggestions();
                }
            }
            KeyCode::Char('d') if ctrl && self.line.is_empty() => self.ask_to_quit(),
            KeyCode::Tab => {
                let chosen = self.selected.unwrap_or(0);
                if let Some(suggestion) = self.suggestions.get(chosen) {
                    let completion = suggestion.completion.clone();
                    self.line.set(&completion);
                    self.refresh_suggestions();
                }
            }
            KeyCode::Down if !self.suggestions.is_empty() => {
                let count = self.suggestions.len();
                self.selected = Some(self.selected.map_or(0, |i| (i + 1) % count));
            }
            KeyCode::Up if !self.suggestions.is_empty() => {
                let count = self.suggestions.len();
                self.selected = Some(self.selected.map_or(count - 1, |i| (i + count - 1) % count));
            }
            KeyCode::Left => self.line.left(),
            KeyCode::Right => self.line.right(),
            KeyCode::Home => self.line.home(),
            KeyCode::End => self.line.end(),
            KeyCode::Char('a') if ctrl => self.line.home(),
            KeyCode::Char('e') if ctrl => self.line.end(),
            KeyCode::Char('u') if ctrl => {
                self.line.take();
                self.refresh_suggestions();
            }
            KeyCode::Char('w') if ctrl => {
                self.line.delete_word();
                self.refresh_suggestions();
            }
            KeyCode::Backspace => {
                self.line.backspace();
                self.refresh_suggestions();
            }
            KeyCode::Delete => {
                self.line.delete();
                self.refresh_suggestions();
            }
            KeyCode::Char(c) if !ctrl => {
                self.line.insert(c.encode_utf8(&mut [0; 4]));
                self.refresh_suggestions();
            }
            _ => {}
        }
    }

    // A space

    fn space_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let prefix = ctrl && key.code == KeyCode::Char(PREFIX);
        // A message from opening the space shows until the next key.
        self.message = None;
        match self.mode {
            Mode::Prefix => {
                self.mode = Mode::Normal;
                match key.code {
                    KeyCode::Char('h') => self.show_welcome(),
                    KeyCode::Char('q') => self.ask_to_quit(),
                    KeyCode::Char('s') | KeyCode::PageUp => self.scroll(Scroll::PageUp),
                    // Ctrl-g twice: the shell gets one.
                    _ if prefix => {
                        if let Some(pane) = self.current_pane() {
                            pane.write(vec![0x07]);
                        }
                    }
                    _ => {}
                }
            }
            Mode::Scroll => match key.code {
                KeyCode::Up | KeyCode::Char('k') => self.scroll(Scroll::Delta(1)),
                KeyCode::Down | KeyCode::Char('j') => self.scroll(Scroll::Delta(-1)),
                KeyCode::PageUp | KeyCode::Char('u' | 'b') => self.scroll(Scroll::PageUp),
                KeyCode::PageDown | KeyCode::Char('d' | ' ') => self.scroll(Scroll::PageDown),
                KeyCode::Home | KeyCode::Char('g') if !ctrl => self.scroll(Scroll::Top),
                KeyCode::End | KeyCode::Char('G') => self.scroll(Scroll::Bottom),
                _ => {
                    // Esc, q or anything else: back to the bottom, and to typing.
                    if let Some(pane) = self.current_pane_mut() {
                        pane.scroll(Scroll::Bottom);
                    }
                    self.mode = Mode::Normal;
                }
            },
            Mode::Normal if prefix => self.mode = Mode::Prefix,
            Mode::Normal => {
                let Some(pane) = self.current_pane() else {
                    return;
                };
                if pane.exit().is_some() {
                    // The shell has ended: Enter starts another one.
                    if key.code == KeyCode::Enter {
                        self.restart_shell();
                    }
                    return;
                }
                let mode = pane.mode();
                // Shift-PageUp reads the scrollback, where the terminal passes it on
                // and the program has no screen of its own.
                if key.code == KeyCode::PageUp
                    && key.modifiers.contains(KeyModifiers::SHIFT)
                    && !mode.contains(TermMode::ALT_SCREEN)
                {
                    self.scroll(Scroll::PageUp);
                    return;
                }
                if let Some(bytes) = keys::encode(&key, mode.contains(TermMode::APP_CURSOR)) {
                    pane.write(bytes);
                }
            }
        }
    }

    /// Moves the view through the scrollback; keys move it until Esc.
    fn scroll(&mut self, scroll: Scroll) {
        if let Some(pane) = self.current_pane_mut() {
            pane.scroll(scroll);
            self.mode = Mode::Scroll;
        }
    }

    fn paste(&mut self, text: &str) {
        match self.screen {
            Screen::Welcome => {
                self.line.insert(text);
                self.refresh_suggestions();
            }
            Screen::Space if self.mode == Mode::Normal => {
                if let Some(pane) = self.current_pane() {
                    let bracketed = pane.mode().contains(TermMode::BRACKETED_PASTE);
                    pane.write(keys::paste(text, bracketed));
                }
            }
            Screen::Space => {}
        }
    }

    // Quitting

    /// Quits, after asking if a program is still running in some space.
    fn ask_to_quit(&mut self) {
        let busy: Vec<String> = self
            .panes
            .iter()
            .filter(|p| p.pane.is_busy())
            .map(|p| p.space.name.clone())
            .collect();
        if busy.is_empty() {
            self.quit = true;
        } else {
            self.quit_question = Some(QuitQuestion { busy });
        }
    }

    fn quit_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('y' | 'Y') => self.quit = true,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => self.quit_question = None,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit_question = None;
            }
            _ => {}
        }
    }

    /// Hangs up every program, and kills what does not exit.
    pub fn shut_down(&mut self) {
        self.panes.clear();
        self.sessions.shutdown(SHUTDOWN_GRACE);
    }

    // Messages

    pub fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Input(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                if self.quit_question.is_some() {
                    self.quit_key(key);
                } else {
                    match self.screen {
                        Screen::Welcome => self.welcome_key(key),
                        Screen::Space => self.space_key(key),
                    }
                }
            }
            Msg::Input(Event::Paste(text)) => self.paste(&text),
            Msg::Input(Event::Resize(cols, rows)) => {
                self.size = (cols, rows);
                let size = self.pane_size();
                for p in &mut self.panes {
                    p.pane.resize(size);
                }
            }
            Msg::Input(_) => {}
            Msg::InputClosed(error) => {
                self.failure = Some(format!("could not read the terminal: {error}"));
                self.quit = true;
            }
            Msg::Output(id, bytes) => {
                if let Some(p) = self.panes.iter_mut().find(|p| p.pane.id == id) {
                    p.pane.feed(&bytes);
                }
            }
            Msg::PaneError(id, error) => {
                if let Some(p) = self.panes.iter().find(|p| p.pane.id == id) {
                    let name = p.space.name.clone();
                    self.complain(format!("{name}'s shell: {error}"));
                }
            }
            Msg::Exited(id, exit) => {
                if let Some(p) = self.panes.iter_mut().find(|p| p.pane.id == id) {
                    p.pane.exited(exit);
                    let _ = self.sessions.close(p.pane.session_id());
                }
            }
        }
    }

    /// The soonest a pane's synchronized update must be shown.
    fn sync_deadline(&self) -> Option<Instant> {
        self.panes
            .iter()
            .filter_map(|p| p.pane.sync_deadline())
            .min()
    }

    fn end_due_syncs(&mut self) {
        let now = Instant::now();
        for p in &mut self.panes {
            if p.pane.sync_deadline().is_some_and(|due| due <= now) {
                p.pane.end_sync();
            }
        }
    }

    /// The cursor the user's terminal should show: the shell's own, in a space.
    fn wanted_cursor(&self) -> Option<(CursorShape, bool)> {
        match (self.screen, self.mode, self.quit_question.as_ref()) {
            (Screen::Space, Mode::Normal | Mode::Prefix, None) => {
                self.current_pane().and_then(Pane::cursor_style)
            }
            _ => None,
        }
    }

    /// Shows the shell's cursor shape (a bar in vim's insert mode, say), and
    /// the user's own shape elsewhere. Sent only when it changes.
    fn apply_cursor(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        let wanted = self.wanted_cursor();
        if wanted == self.cursor {
            return Ok(());
        }
        self.cursor = wanted;
        let style = match wanted {
            // The block that does not blink is the default; the user's own
            // shape stands in for it.
            None
            | Some((CursorShape::Block | CursorShape::HollowBlock | CursorShape::Hidden, false)) => {
                SetCursorStyle::DefaultUserShape
            }
            Some((CursorShape::Block | CursorShape::HollowBlock | CursorShape::Hidden, true)) => {
                SetCursorStyle::BlinkingBlock
            }
            Some((CursorShape::Underline, true)) => SetCursorStyle::BlinkingUnderScore,
            Some((CursorShape::Underline, false)) => SetCursorStyle::SteadyUnderScore,
            Some((CursorShape::Beam, true)) => SetCursorStyle::BlinkingBar,
            Some((CursorShape::Beam, false)) => SetCursorStyle::SteadyBar,
        };
        execute!(terminal.backend_mut(), style)
    }
}

/// Reads keys on a thread of its own, so the loop can wait for keys and output
/// together.
pub fn read_input(tx: Sender<Msg>) {
    std::thread::spawn(move || {
        loop {
            match event::read() {
                Ok(event) => {
                    if tx.send(Msg::Input(event)).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let _ = tx.send(Msg::InputClosed(error.to_string()));
                    return;
                }
            }
        }
    });
}

/// Draws, waits for something to happen, handles it and everything else
/// already waiting, and draws again, until the user quits.
pub fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    rx: &Receiver<Msg>,
) -> std::io::Result<()> {
    loop {
        terminal.draw(|frame: &mut Frame<'_>| ui::draw(frame, app))?;
        app.apply_cursor(terminal)?;
        if app.quit {
            return Ok(());
        }
        let first = match app.sync_deadline() {
            Some(due) => match rx.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(msg) => Some(msg),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return Ok(()),
            },
            None => match rx.recv() {
                Ok(msg) => Some(msg),
                Err(mpsc::RecvError) => return Ok(()),
            },
        };
        let started = Instant::now();
        if let Some(msg) = first {
            app.handle(msg);
        }
        while started.elapsed() < FRAME_BUDGET && !app.quit {
            match rx.try_recv() {
                Ok(msg) => app.handle(msg),
                Err(_) => break,
            }
        }
        app.end_due_syncs();
    }
}
