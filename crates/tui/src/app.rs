//! The app: the Welcome screen and the open space, what each key and mouse
//! action does there, and the loop that reads them and programs' output.
//!
//! One space is shown at a time, as in the desktop app. Each space opened in
//! this run keeps its tabs and panes while `x8ai` runs, so going back to it
//! with `/cd` finds them as they were left.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::CursorShape;
use ratatui::crossterm::cursor::SetCursorStyle;
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
    MouseEventKind,
};
use ratatui::crossterm::execute;
use ratatui::layout::{Position, Rect};
use ratatui::{DefaultTerminal, Frame};
use x8ai_agents::LaunchPlan;
use x8ai_core::agent::AgentSessionId;
use x8ai_core::terminal::{TerminalExit, TerminalSize};
use x8ai_core::workspace::RecentWorkspace;
use x8ai_pty::{Environment, Program, Sessions};

use crate::agents::Agents;
use crate::clipboard;
use crate::files::Activated;
use crate::keys;
use crate::layout::{self, Axis, Direction, Divider};
use crate::mouse::{Action, Button};
use crate::pane::{Pane, PaneId};
use crate::space::{Focus, Kind, Label, Sidebar, Slot, SpaceLayout, SpaceView};
use crate::spaces::{SpaceInfo, Spaces};
use crate::theme::Theme;
use crate::ui;
use crate::welcome::{self, Command, Context, Line, MAX_NAME_LENGTH, Suggestion};

/// How long programs get to exit after hangup when `x8ai` quits.
const SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// The longest the loop handles messages before drawing, so a flood of output
/// still shows as it arrives.
const FRAME_BUDGET: Duration = Duration::from_millis(16);

/// How the user's editor is started on a file, as git starts it: through
/// `sh`, so `$VISUAL` or `$EDITOR` may carry options (`code -w`). The file is
/// an argument (`$1`), never part of the script.
const EDITOR_SCRIPT: &str = r#"exec ${VISUAL:-${EDITOR:-vi}} "$1""#;

/// Lines the mouse wheel moves per notch.
const WHEEL_LINES: i32 = 3;

/// What reaches the loop: keys and the mouse from the user's terminal,
/// programs' output, and changes to a space's files.
pub enum Msg {
    Input(Event),
    /// Reading the user's terminal failed; nothing more can be typed.
    InputClosed(String),
    Output(PaneId, Vec<u8>),
    PaneError(PaneId, String),
    Exited(PaneId, TerminalExit),
    /// Something changed in this space's folder.
    FilesChanged(PathBuf),
    /// What looking for local models (Ollama) found.
    Local(x8ai_providers::ollama::Detection),
}

mod agent_panel;
mod panels;

pub use panels::{Dialog, Draft};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Welcome,
    Space,
}

/// What keys do in a space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Keys go to the focused pane or the file list; Ctrl-g is the prefix
    /// for x8ai's own keys.
    Normal,
    /// Ctrl-g was pressed: the next key is x8ai's.
    Prefix,
    /// Reading the scrollback: keys move the view.
    Scroll,
    /// Every key, listed; any key closes it.
    Help,
}

/// Ctrl-g: the prefix for x8ai's keys in a space. Not Ctrl-Space, which macOS
/// keeps for switching input sources, nor Ctrl-b (tmux) or Ctrl-a (readline).
const PREFIX: char = 'g';

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub text: String,
    pub error: bool,
}

/// What a question asks to do, on y.
#[derive(Debug, Clone)]
pub enum Ask {
    Quit,
    ClosePane(PaneId),
    /// Trust the folder, then go on.
    Trust(PathBuf, Option<Then>),
    /// Stop trusting the folder: its approvals go, its agents stop.
    Untrust(PathBuf),
    /// Allow this launch in its folder (the agent, if `true`), and these MCP
    /// servers, then go on.
    Approve(Box<LaunchPlan>, bool, Vec<x8ai_mcp::Prepared>, Then),
    /// Remove a session and its worktree; with `true`, its uncommitted
    /// changes go too.
    Remove(AgentSessionId, bool),
    /// Delete a provider's key from the Keychain.
    RemoveKey(String),
    RemoveMcp(String),
    RemoveSkill(String),
    /// Install add-ons in a tab of their own, then add them to the space of
    /// `root`.
    Install {
        addons: Vec<&'static str>,
        root: Option<PathBuf>,
        name: String,
        script: String,
    },
}

/// What an agent's launch goes on with once a question is answered yes.
#[derive(Debug, Clone)]
pub enum Then {
    /// A new session of this agent.
    Launch(String),
    /// This session's agent, again.
    Run(AgentSessionId),
}

/// A question in a box: y does `ask`, n or Esc does nothing.
pub struct Question {
    pub ask: Ask,
    pub title: String,
    pub lines: Vec<String>,
    /// What y does, in a word or two.
    pub yes: &'static str,
}

/// What a mouse button held down is doing.
#[derive(Debug, Clone, Copy)]
enum Drag {
    /// Moving the line between two panes.
    Divider(Divider),
    /// Selecting text in a pane, whose body is given.
    Select(PaneId, Rect),
    /// Held over a program that asked for the mouse.
    Report(PaneId, Rect, Button),
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
    /// The open space; the Welcome screen shows over it.
    pub current: SpaceInfo,
    pub question: Option<Question>,
    /// A form or a picker on top.
    pub dialog: Option<Dialog>,
    /// Why `x8ai` stopped, when it was not the user's choice.
    pub failure: Option<String>,
    recent: Vec<RecentWorkspace>,
    chosen_name: Option<String>,
    account_name: Option<String>,
    /// Each space shown in this run, with its tabs and panes.
    views: Vec<SpaceView>,
    agents: Agents,
    services: crate::services::Services,
    /// What each agent's next launch gets, by agent id.
    drafts: std::collections::HashMap<String, Draft>,
    /// The catalog's items as the Catalog panel last read them.
    catalog: Vec<x8ai_core::catalog::CatalogItem>,
    /// Agents whose panes were closed while they ran: still ending.
    ending: Vec<AgentSessionId>,
    spaces: Spaces,
    sessions: Sessions,
    tx: Sender<Msg>,
    cwd: PathBuf,
    next_pane: u32,
    next_split: u32,
    size: (u16, u16),
    cursor: Option<(CursorShape, bool)>,
    drag: Option<Drag>,
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
        let agents = Agents::new(spaces.home(), std::env::vars().collect());
        let services = crate::services::Services::new(spaces.data_dir().to_owned(), spaces.home());
        let mut app = Self {
            theme: Theme::detect(),
            screen: Screen::Welcome,
            mode: Mode::Normal,
            line: Line::default(),
            selected: None,
            suggestions: Vec::new(),
            message: None,
            current,
            question: None,
            dialog: None,
            failure: None,
            recent: Vec::new(),
            chosen_name: spaces.chosen_name(),
            account_name: crate::spaces::account_name(),
            views: Vec::new(),
            agents,
            services,
            drafts: std::collections::HashMap::new(),
            catalog: Vec::new(),
            ending: Vec::new(),
            spaces,
            sessions: Sessions::default(),
            tx,
            cwd,
            next_pane: 0,
            next_split: 0,
            size,
            cursor: None,
            drag: None,
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

    /// The open space, once it has been shown.
    pub fn view(&self) -> Option<&SpaceView> {
        self.views.iter().find(|v| v.info.root == self.current.root)
    }

    fn view_at(&self) -> Option<usize> {
        self.views
            .iter()
            .position(|v| v.info.root == self.current.root)
    }

    fn area(&self) -> Rect {
        Rect::new(0, 0, self.size.0, self.size.1)
    }

    /// Where everything in the open space is on the screen.
    pub fn space_layout(&self) -> Option<SpaceLayout> {
        self.view().map(|v| v.layout(self.area()))
    }

    fn slot_mut(&mut self, id: PaneId) -> Option<&mut Slot> {
        self.views.iter_mut().find_map(|v| v.slot_mut(id))
    }

    // Messages

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

    // Welcome

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
        self.drag = None;
        self.refresh_recent();
    }

    /// The open space, with a shell started if it has no terminal open.
    fn show_space(&mut self) {
        let at = self.view_at().unwrap_or_else(|| {
            let place = self.current.root.as_deref().map_or_else(
                || "~".to_owned(),
                |root| welcome::tilde(root, self.spaces.home()),
            );
            self.views
                .push(SpaceView::new(self.current.clone(), place, &self.tx));
            self.views.len() - 1
        });
        // What is known about the space now (its trust may have changed).
        self.views[at].info = self.current.clone();
        if self.views[at].tabs.is_empty() {
            match self.start_shell(self.new_pane_size(at, None)) {
                Ok(slot) => self.views[at].add_tab(slot),
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
                self.complain("Name the new space: /new <name>. It is made in ~/Workspaces.");
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
            Command::Share("") => self.complain("Name the space to share with: /share <space>."),
            Command::Share(name) => self.share_with(name),
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

    // Panes and tabs

    /// Starts a program for a new pane of `size`.
    fn start(&mut self, program: &Program, kind: Kind, size: TerminalSize) -> Result<Slot, String> {
        let id = PaneId(self.next_pane);
        self.next_pane += 1;
        let pane = Pane::start(id, &self.sessions, program, size, self.tx.clone())?;
        Ok(Slot { pane, kind })
    }

    /// The user's login shell in the open space's folder, with its id, as the
    /// app's terminals have it (ADR 0019).
    fn start_shell(&mut self, size: TerminalSize) -> Result<Slot, String> {
        let program = Program::LoginShell {
            cwd: self.current.root.clone(),
            env: self.shell_env(),
        };
        self.start(&program, Kind::Shell, size)
    }

    /// What a new shell of the open space starts with: its id, and its
    /// add-ons (ADR 0019), exactly as the app's terminals have them.
    fn shell_env(&mut self) -> Vec<(String, String)> {
        let root = self.current.root.clone();
        let space = match self.spaces.space(root.as_deref()) {
            Ok(space) => space,
            Err(_) => {
                return self
                    .current
                    .id
                    .iter()
                    .map(|id| ("X8AI_SPACE".to_owned(), id.clone()))
                    .collect();
            }
        };
        let trusted = root.as_deref().is_none_or(|r| self.spaces.is_trusted(r));
        let title = format!("{} ({})", self.current.name, space.id);
        let (env, problem) = x8ai_addons::terminal_env(
            &x8ai_addons::SpaceTerminal {
                id: &space.id,
                title: &title,
                addons: &space.addons,
                trusted,
            },
            &self.mac(),
            x8ai_addons::is_zsh(&x8ai_pty::user_shell()),
            self.spaces.data_dir(),
        );
        if let Some(problem) = problem {
            self.complain(problem);
        }
        env
    }

    /// The size a new pane in view `at` gets: a tab of its own, or half the
    /// focused pane.
    fn new_pane_size(&self, at: usize, split: Option<Axis>) -> TerminalSize {
        self.views[at].new_pane_size(self.area(), split)
    }

    fn new_tab(&mut self) {
        let Some(at) = self.view_at() else {
            return;
        };
        match self.start_shell(self.new_pane_size(at, None)) {
            Ok(slot) => self.views[at].add_tab(slot),
            Err(error) => self.complain(format!("Could not start a shell: {error}")),
        }
    }

    fn split(&mut self, axis: Axis) {
        let Some(at) = self.view_at() else {
            return;
        };
        match self.start_shell(self.new_pane_size(at, Some(axis))) {
            Ok(slot) => {
                self.next_split += 1;
                self.views[at].split(slot, axis, self.next_split);
            }
            Err(error) => self.complain(format!("Could not start a shell: {error}")),
        }
    }

    /// Opens `path` in the user's editor, in a tab of its own; a tab that
    /// already has it open is shown instead.
    fn open_in_editor(&mut self, path: PathBuf) {
        let Some(at) = self.view_at() else {
            return;
        };
        if let Some(tab) = self.views[at].editor_tab(&path) {
            self.views[at].show_tab(tab);
            return;
        }
        let program = Program::Exec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                EDITOR_SCRIPT.into(),
                "x8ai-editor".into(),
                path.clone().into_os_string(),
            ],
            cwd: self.current.root.clone(),
            env: Environment::Inherit,
        };
        let size = self.new_pane_size(at, None);
        match self.start(&program, Kind::Editor(path), size) {
            Ok(slot) => self.views[at].add_tab(slot),
            Err(error) => self.complain(format!("Could not start your editor: {error}")),
        }
    }

    /// Closes pane `id`, after asking if a program is still running in it.
    fn ask_to_close(&mut self, id: PaneId) {
        let Some(slot) = self.view().and_then(|v| v.slot(id)) else {
            return;
        };
        if slot.is_busy() {
            let title = slot.title();
            let (what, yes) = match slot.kind {
                Kind::Agent(..) => (format!("Stop {title}?"), "stop"),
                _ => ("Close this pane?".to_owned(), "close"),
            };
            self.question = Some(Question {
                ask: Ask::ClosePane(id),
                title: what,
                lines: vec![format!(
                    "{title} is still running. Closing the pane stops it."
                )],
                yes,
            });
        } else {
            self.close(id);
        }
    }

    /// Closes pane `id`, wherever it is. When the space on screen is left
    /// with no terminal, the Welcome shows.
    fn close(&mut self, id: PaneId) {
        let Some(at) = self.views.iter().position(|v| v.slot(id).is_some()) else {
            return;
        };
        if let Some(slot) = self.views[at].remove(id) {
            if let Kind::Agent(session, _) = slot.kind
                && slot.pane.exit().is_none()
            {
                self.ending.push(session);
            }
            let _ = self.sessions.close(slot.pane.session_id());
        }
        self.drag = None;
        let view = &self.views[at];
        if view.tabs.is_empty()
            && view.info.root == self.current.root
            && self.screen == Screen::Space
        {
            let name = view.info.name.clone();
            self.show_welcome();
            self.say(format!(
                "{name} has no terminal open. Press Esc to start a new one."
            ));
        }
    }

    /// A shell that ended badly: another one in its place.
    fn restart(&mut self, id: PaneId) {
        let (Some(at), Some(layout)) = (self.view_at(), self.space_layout()) else {
            return;
        };
        let size = layout.panes.area_of(id).map_or_else(
            || self.new_pane_size(at, None),
            |area| TerminalSize {
                cols: area.body.width.max(1),
                rows: area.body.height.max(1),
            },
        );
        match self.start_shell(size) {
            Ok(slot) => {
                if let Some(old) = self.views[at].replace(id, slot) {
                    let _ = self.sessions.close(old.pane.session_id());
                }
            }
            Err(error) => self.complain(format!("Could not start a shell: {error}")),
        }
    }

    fn move_focus(&mut self, direction: Direction) {
        let (Some(at), Some(layout)) = (self.view_at(), self.space_layout()) else {
            return;
        };
        let view = &mut self.views[at];
        if view.focus == Focus::Sidebar {
            if direction == Direction::Right {
                view.focus = Focus::Panes;
            }
            return;
        }
        let Some(from) = view.focused() else {
            return;
        };
        match layout.panes.neighbor(from, direction) {
            Some(to) => {
                if let Some(tab) = view.tab_mut() {
                    tab.focused = to;
                }
            }
            None if direction == Direction::Left && layout.sidebar.is_some() => {
                view.focus = Focus::Sidebar;
            }
            None => {}
        }
    }

    /// Ctrl-g o: the next pane of the tab, in reading order.
    fn next_pane(&mut self) {
        let Some(at) = self.view_at() else {
            return;
        };
        let view = &mut self.views[at];
        view.focus = Focus::Panes;
        if let Some(tab) = view.tab_mut() {
            let panes = tab.tree.panes();
            let here = panes.iter().position(|&p| p == tab.focused).unwrap_or(0);
            tab.focused = panes[(here + 1) % panes.len()];
            tab.zoomed = false;
        }
    }

    /// Ctrl-g f: shows the file list and gives it the keys; hides it when it
    /// has them.
    fn toggle_files(&mut self) {
        let Some(at) = self.view_at() else {
            return;
        };
        if self.views[at].files.is_none() {
            self.say(if self.current.root.is_none() {
                "The workspace with no folder has no file list."
            } else {
                "This folder cannot be read."
            });
            return;
        }
        self.toggle_sidebar(at, Sidebar::Files);
    }

    /// Shows `sidebar` and gives it the keys; hides it when it has them.
    fn toggle_sidebar(&mut self, at: usize, sidebar: Sidebar) {
        let view = &mut self.views[at];
        if view.sidebar == sidebar && view.focus == Focus::Sidebar {
            view.sidebar = Sidebar::Hidden;
            view.focus = Focus::Panes;
        } else if self.size.0 < crate::space::MIN_WIDTH_FOR_SIDEBAR {
            self.say("Make the window wider to show the sidebar.");
        } else {
            view.sidebar = sidebar;
            view.focus = Focus::Sidebar;
        }
    }

    // Keys in a space

    fn space_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let prefix = ctrl && key.code == KeyCode::Char(PREFIX);
        // A message shows until the next key.
        self.message = None;
        match self.mode {
            Mode::Help => self.mode = Mode::Normal,
            Mode::Prefix => {
                self.mode = Mode::Normal;
                self.prefix_key(key, prefix);
            }
            Mode::Scroll => self.scroll_key(key),
            Mode::Normal if prefix => self.mode = Mode::Prefix,
            Mode::Normal => match self.view().map(|v| (v.focus, v.sidebar)) {
                Some((Focus::Sidebar, Sidebar::Files)) => self.files_key(key),
                Some((Focus::Sidebar, Sidebar::Agents)) => self.agents_key(key),
                Some((Focus::Sidebar, sidebar)) if sidebar.is_list() => self.list_key(key),
                Some(_) => self.pane_key(key),
                None => {}
            },
        }
    }

    /// The key after Ctrl-g.
    fn prefix_key(&mut self, key: KeyEvent, prefix: bool) {
        let Some(at) = self.view_at() else {
            return;
        };
        match key.code {
            KeyCode::Char('h') => self.show_welcome(),
            KeyCode::Char('q') => self.ask_to_quit(),
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('s') | KeyCode::PageUp => self.scroll(Scroll::PageUp),
            KeyCode::Char('t' | 'c') => self.new_tab(),
            KeyCode::Char('n' | 'p') => {
                let view = &mut self.views[at];
                let count = view.tabs.len();
                if count > 0 {
                    let step = if key.code == KeyCode::Char('n') {
                        1
                    } else {
                        count - 1
                    };
                    view.show_tab((view.active + step) % count);
                }
            }
            KeyCode::Char(c @ '1'..='9') => {
                self.views[at].show_tab(usize::from(c as u8 - b'1'));
            }
            KeyCode::Char('|' | '%' | '\\') => self.split(Axis::Right),
            KeyCode::Char('-' | '"') => self.split(Axis::Down),
            KeyCode::Char('x') => {
                if let Some(id) = self.views[at].focused() {
                    self.ask_to_close(id);
                }
            }
            KeyCode::Char('o') => self.next_pane(),
            KeyCode::Left => self.move_focus(Direction::Left),
            KeyCode::Right => self.move_focus(Direction::Right),
            KeyCode::Up => self.move_focus(Direction::Up),
            KeyCode::Down => self.move_focus(Direction::Down),
            KeyCode::Char('z') => {
                if let Some(tab) = self.views[at].tab_mut() {
                    tab.zoomed = !tab.zoomed;
                }
            }
            KeyCode::Char('f') => self.toggle_files(),
            KeyCode::Char('a') => self.toggle_agents(),
            KeyCode::Char('m') => self.toggle_list(Sidebar::Models),
            KeyCode::Char('u') => self.toggle_list(Sidebar::Mcp),
            KeyCode::Char('k') => self.toggle_list(Sidebar::Catalog),
            KeyCode::Char('e') => self.toggle_list(Sidebar::Addons),
            // Ctrl-g twice: the program gets one.
            _ if prefix => {
                if let Some(slot) = self.views[at].focused_slot() {
                    slot.pane.write(vec![0x07]);
                }
            }
            _ => {}
        }
    }

    /// Keys for the focused pane's program.
    fn pane_key(&mut self, key: KeyEvent) {
        let Some(at) = self.view_at() else {
            return;
        };
        let Some(slot) = self.views[at].focused_slot() else {
            return;
        };
        let (id, mode) = (slot.pane.id, slot.pane.mode());
        if slot.pane.exit().is_some() {
            // Ended: Enter runs the agent again, starts another shell (one that
            // failed), or closes the pane.
            if key.code == KeyCode::Enter {
                match slot.kind {
                    Kind::Agent(session, _) => self.run_session(session, Some(id)),
                    Kind::Shell => self.restart(id),
                    Kind::Editor(_) | Kind::Review(..) | Kind::Install { .. } => self.close(id),
                }
            }
            return;
        }
        // Shift-PageUp reads the scrollback, where the terminal passes it on and
        // the program has no screen of its own.
        if key.code == KeyCode::PageUp
            && key.modifiers.contains(KeyModifiers::SHIFT)
            && !mode.contains(TermMode::ALT_SCREEN)
        {
            self.scroll(Scroll::PageUp);
            return;
        }
        let Some(slot) = self.views[at].focused_slot_mut() else {
            return;
        };
        slot.pane.clear_selection();
        if slot.pane.scrolled_back() > 0 {
            slot.pane.scroll(Scroll::Bottom);
        }
        if let Some(bytes) = keys::encode(&key, mode.contains(TermMode::APP_CURSOR)) {
            slot.pane.write(bytes);
        }
    }

    /// Keys for the file list.
    fn files_key(&mut self, key: KeyEvent) {
        let (Some(at), Some(layout)) = (self.view_at(), self.space_layout()) else {
            return;
        };
        let height = layout
            .sidebar_rows()
            .map_or(1, |r| usize::from(r.height).max(1));
        let page = isize::try_from(height.saturating_sub(1).max(1)).unwrap_or(1);
        let view = &mut self.views[at];
        let Some(files) = view.files.as_mut() else {
            view.focus = Focus::Panes;
            return;
        };
        let mut open = None;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => files.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => files.move_by(1),
            KeyCode::PageUp => files.move_by(-page),
            KeyCode::PageDown => files.move_by(page),
            KeyCode::Home | KeyCode::Char('g') => files.first(),
            KeyCode::End | KeyCode::Char('G') => files.last(),
            KeyCode::Right | KeyCode::Char('l') => files.expand(),
            KeyCode::Left | KeyCode::Char('h') => files.collapse(),
            KeyCode::Enter => {
                if let Activated::File(path) = files.activate() {
                    open = Some(path);
                }
            }
            KeyCode::Esc | KeyCode::Tab => view.focus = Focus::Panes,
            _ => {}
        }
        if let Some(files) = view.files.as_mut() {
            files.keep_in_view(height);
        }
        if let Some(path) = open {
            self.open_in_editor(path);
        }
    }

    /// Moves the focused pane's view through its scrollback; keys move it
    /// until Esc.
    fn scroll(&mut self, scroll: Scroll) {
        let Some(at) = self.view_at() else {
            return;
        };
        if let Some(slot) = self.views[at].focused_slot_mut() {
            slot.pane.scroll(scroll);
            self.mode = Mode::Scroll;
        }
    }

    fn scroll_key(&mut self, key: KeyEvent) {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.scroll(Scroll::Delta(1)),
            KeyCode::Down | KeyCode::Char('j') => self.scroll(Scroll::Delta(-1)),
            KeyCode::PageUp | KeyCode::Char('u' | 'b') => self.scroll(Scroll::PageUp),
            KeyCode::PageDown | KeyCode::Char('d' | ' ') => self.scroll(Scroll::PageDown),
            KeyCode::Home | KeyCode::Char('g') if !ctrl => self.scroll(Scroll::Top),
            KeyCode::End | KeyCode::Char('G') => self.scroll(Scroll::Bottom),
            _ => {
                // Esc, q or anything else: back to the bottom, and to typing.
                if let Some(at) = self.view_at()
                    && let Some(slot) = self.views[at].focused_slot_mut()
                {
                    slot.pane.scroll(Scroll::Bottom);
                }
                self.mode = Mode::Normal;
            }
        }
    }

    fn paste(&mut self, text: &str) {
        match self.screen {
            Screen::Welcome => {
                self.line.insert(text);
                self.refresh_suggestions();
            }
            Screen::Space if self.mode == Mode::Normal => {
                let Some(at) = self.view_at() else {
                    return;
                };
                if self.views[at].focus != Focus::Panes {
                    return;
                }
                if let Some(slot) = self.views[at].focused_slot() {
                    let bracketed = slot.pane.mode().contains(TermMode::BRACKETED_PASTE);
                    slot.pane.write(keys::paste(text, bracketed));
                }
            }
            Screen::Space => {}
        }
    }

    // The mouse

    fn mouse(&mut self, event: MouseEvent) {
        if self.screen != Screen::Space || self.question.is_some() || self.dialog.is_some() {
            return;
        }
        if self.mode == Mode::Help {
            if matches!(event.kind, MouseEventKind::Down(_)) {
                self.mode = Mode::Normal;
            }
            return;
        }
        let (Some(at), Some(layout)) = (self.view_at(), self.space_layout()) else {
            return;
        };
        let at_cell = Position::new(event.column, event.row);
        let shift = event.modifiers.contains(KeyModifiers::SHIFT);
        match event.kind {
            MouseEventKind::Down(pressed) => {
                self.message = None;
                if self.mode == Mode::Prefix {
                    self.mode = Mode::Normal;
                }
                if pressed == MouseButton::Left && self.click_chrome(at, &layout, at_cell) {
                    return;
                }
                let Some(area) = layout.panes.pane_at(at_cell.x, at_cell.y).copied() else {
                    return;
                };
                let view = &mut self.views[at];
                view.focus = Focus::Panes;
                if let Some(tab) = view.tab_mut() {
                    tab.focused = area.id;
                }
                for slot in &mut view.slots {
                    slot.pane.clear_selection();
                }
                if !area.body.contains(at_cell) {
                    return;
                }
                let cell = inside(area.body, at_cell);
                let button = match pressed {
                    MouseButton::Left => Button::Left,
                    MouseButton::Middle => Button::Middle,
                    MouseButton::Right => Button::Right,
                };
                let Some(slot) = view.slot_mut(area.id) else {
                    return;
                };
                if slot.pane.wants_mouse() && !shift {
                    slot.pane
                        .report_mouse(Action::Press, button, cell, event.modifiers);
                    self.drag = Some(Drag::Report(area.id, area.body, button));
                } else if button == Button::Left {
                    slot.pane.start_selection(cell.0, cell.1);
                    self.drag = Some(Drag::Select(area.id, area.body));
                }
            }
            MouseEventKind::Drag(_) => match self.drag {
                Some(Drag::Divider(divider)) => {
                    if let Some(tab) = self.views[at].tab_mut() {
                        let ratio = layout::ratio_at(&divider, at_cell.x, at_cell.y);
                        tab.tree.resize(divider.split, ratio);
                    }
                }
                Some(Drag::Select(id, body)) => {
                    if let Some(slot) = self.slot_mut(id) {
                        let (col, row) = inside(body, at_cell);
                        slot.pane.extend_selection(col, row);
                    }
                }
                Some(Drag::Report(id, body, button)) => {
                    if let Some(slot) = self.slot_mut(id) {
                        let cell = inside(body, at_cell);
                        slot.pane
                            .report_mouse(Action::Drag, button, cell, event.modifiers);
                    }
                }
                None => {}
            },
            MouseEventKind::Up(_) => match self.drag.take() {
                Some(Drag::Select(id, _)) => {
                    let text = self.slot_mut(id).and_then(|s| s.pane.end_selection());
                    if let Some(text) = text {
                        match clipboard::copy(&text) {
                            Ok(()) => {
                                self.say(format!("Copied {} characters.", text.chars().count()))
                            }
                            Err(error) => self.complain(error),
                        }
                    }
                }
                Some(Drag::Report(id, body, button)) => {
                    if let Some(slot) = self.slot_mut(id) {
                        let cell = inside(body, at_cell);
                        slot.pane
                            .report_mouse(Action::Release, button, cell, event.modifiers);
                    }
                }
                _ => {}
            },
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let up = event.kind == MouseEventKind::ScrollUp;
                self.wheel(at, &layout, at_cell, up, event.modifiers);
            }
            _ => {}
        }
    }

    /// A left click on the header's tabs, a line between panes, or the file
    /// list. Returns whether it was one of those.
    fn click_chrome(&mut self, at: usize, layout: &SpaceLayout, cell: Position) -> bool {
        if let Some(&(label, _)) = layout.labels.iter().find(|(_, r)| r.contains(cell)) {
            match label {
                Label::Tab(index) => self.views[at].show_tab(index),
                Label::New => self.new_tab(),
            }
            return true;
        }
        if cell.y == layout.header.y {
            return true;
        }
        if let Some(divider) = layout.panes.divider_at(cell.x, cell.y) {
            self.drag = Some(Drag::Divider(*divider));
            // A pane's title row is also the line above it: the pane gets the keys.
            if let Some(area) = layout.panes.pane_at(cell.x, cell.y) {
                let view = &mut self.views[at];
                view.focus = Focus::Panes;
                if let Some(tab) = view.tab_mut() {
                    tab.focused = area.id;
                }
            }
            return true;
        }
        if !layout.sidebar.is_some_and(|f| f.contains(cell)) {
            return false;
        }
        let view = &mut self.views[at];
        view.focus = Focus::Sidebar;
        if view.sidebar == Sidebar::Agents {
            if let Some(rows) = layout.sidebar_rows()
                && rows.contains(cell)
            {
                self.click_panel_row(at, usize::from(cell.y - rows.y));
            }
            return true;
        }
        if view.sidebar.is_list() {
            if let Some(rows) = layout.sidebar_rows()
                && rows.contains(cell)
                && let Some(row) = view.list.row_at(usize::from(cell.y - rows.y))
            {
                view.list.selected = row;
            }
            return true;
        }
        let mut open = None;
        if let (Some(files), Some(rows)) = (view.files.as_mut(), layout.sidebar_rows())
            && rows.contains(cell)
        {
            let row = files.offset() + usize::from(cell.y - rows.y);
            if row < files.rows().len() {
                files.select(row);
                // A folder opens or closes; a file opens in the editor.
                if let Activated::File(path) = files.activate() {
                    open = Some(path);
                }
            }
        }
        if let Some(path) = open {
            self.open_in_editor(path);
        }
        true
    }

    /// The wheel: the file list scrolls; a program that asked for the mouse
    /// gets it; one on its own screen (less, man) gets arrow keys; otherwise
    /// the scrollback moves.
    fn wheel(
        &mut self,
        at: usize,
        layout: &SpaceLayout,
        cell: Position,
        up: bool,
        modifiers: KeyModifiers,
    ) {
        if layout.sidebar.is_some_and(|f| f.contains(cell)) {
            let height = layout.sidebar_rows().map_or(1, |r| usize::from(r.height));
            let view = &mut self.views[at];
            match view.sidebar {
                Sidebar::Files => {
                    if let Some(files) = view.files.as_mut() {
                        files.scroll(if up { -3 } else { 3 }, height);
                    }
                }
                sidebar if sidebar.is_list() => view.list.scroll(if up { -3 } else { 3 }, height),
                _ => {
                    view.panel.move_by(if up { -1 } else { 1 });
                    view.panel.keep_in_view(height);
                }
            }
            return;
        }
        let Some(area) = layout.panes.pane_at(cell.x, cell.y).copied() else {
            return;
        };
        if !area.body.contains(cell) {
            return;
        }
        let Some(slot) = self.views[at].slot_mut(area.id) else {
            return;
        };
        let mode = slot.pane.mode();
        if slot.pane.wants_mouse() && !modifiers.contains(KeyModifiers::SHIFT) {
            let button = if up {
                Button::WheelUp
            } else {
                Button::WheelDown
            };
            slot.pane
                .report_mouse(Action::Press, button, inside(area.body, cell), modifiers);
        } else if mode.contains(TermMode::ALT_SCREEN) && mode.contains(TermMode::ALTERNATE_SCROLL) {
            let arrow = KeyEvent::new(
                if up { KeyCode::Up } else { KeyCode::Down },
                KeyModifiers::NONE,
            );
            if let Some(bytes) = keys::encode(&arrow, mode.contains(TermMode::APP_CURSOR)) {
                slot.pane.write(bytes.repeat(WHEEL_LINES as usize));
            }
        } else {
            slot.pane
                .scroll_lines(if up { WHEEL_LINES } else { -WHEEL_LINES });
        }
    }

    // Quitting

    /// Quits, after asking if a program is still running in some space.
    fn ask_to_quit(&mut self) {
        let busy: Vec<String> = self
            .views
            .iter()
            .flat_map(|v| {
                v.busy()
                    .map(move |s| format!("{} ({})", v.info.name, s.title()))
            })
            .collect();
        if busy.is_empty() {
            self.quit = true;
        } else {
            self.question = Some(Question {
                ask: Ask::Quit,
                title: "Quit x8ai?".to_owned(),
                lines: vec![format!(
                    "Still running: {}. Quitting stops it.",
                    busy.join(", ")
                )],
                yes: "quit",
            });
        }
    }

    fn question_key(&mut self, key: KeyEvent) {
        let yes = match key.code {
            KeyCode::Char('y' | 'Y') => true,
            KeyCode::Char('n' | 'N') | KeyCode::Esc => false,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => false,
            _ => return,
        };
        let Some(question) = self.question.take() else {
            return;
        };
        if !yes {
            return;
        }
        match question.ask {
            Ask::Quit => self.quit = true,
            Ask::ClosePane(id) => self.close(id),
            Ask::Trust(root, then) => self.trust(&root, then),
            Ask::Untrust(root) => self.untrust(&root),
            Ask::Approve(plan, agent, servers, then) => self.approve(&plan, agent, &servers, then),
            Ask::Remove(session, discard) => self.remove_session(session, discard),
            Ask::RemoveKey(provider) => {
                match self.services.remove_key(&provider) {
                    Ok(()) => self.say("The key is deleted from your Keychain."),
                    Err(error) => self.complain(error),
                }
                self.refresh_list();
            }
            Ask::RemoveMcp(id) => {
                match self.services.mcp_remove(&id) {
                    Ok(server) => self.say(format!("{} is removed.", server.name)),
                    Err(error) => self.complain(error),
                }
                self.refresh_list();
            }
            Ask::RemoveSkill(id) => {
                match self.services.skill_remove(&id) {
                    Ok(skill) => self.say(format!("The skill {} is removed.", skill.name)),
                    Err(error) => self.complain(error),
                }
                self.refresh_list();
            }
            Ask::Install {
                addons,
                root,
                name,
                script,
            } => self.install(addons, root, name, script),
        }
    }

    /// Hangs up every program, and kills what does not exit.
    pub fn shut_down(&mut self) {
        self.services.shut_down();
        self.views.clear();
        self.sessions.shutdown(SHUTDOWN_GRACE);
    }

    // The loop

    pub fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Input(Event::Key(key)) if key.kind != KeyEventKind::Release => {
                if self.question.is_some() {
                    self.question_key(key);
                } else if self.dialog.is_some() {
                    self.dialog_key(key);
                } else {
                    match self.screen {
                        Screen::Welcome => self.welcome_key(key),
                        Screen::Space => self.space_key(key),
                    }
                }
            }
            Msg::Input(Event::Mouse(event)) => self.mouse(event),
            Msg::Input(Event::Paste(text)) => match &mut self.dialog {
                Some(Dialog::Form(form)) => form.paste(&text),
                Some(Dialog::Picker(_)) => {}
                None => self.paste(&text),
            },
            Msg::Input(Event::Resize(cols, rows)) => self.size = (cols, rows),
            Msg::Input(_) => {}
            Msg::InputClosed(error) => {
                self.failure = Some(format!("could not read the terminal: {error}"));
                self.quit = true;
            }
            Msg::Output(id, bytes) => {
                if let Some(slot) = self.slot_mut(id) {
                    slot.pane.feed(&bytes);
                }
            }
            Msg::PaneError(id, error) => {
                if let Some(title) = self.views.iter().find_map(|v| v.slot(id)).map(Slot::title) {
                    self.complain(format!("{title}: {error}"));
                }
            }
            Msg::Exited(id, exit) => {
                // A program that ended well takes its pane with it, as closing a
                // terminal tab does; one that failed stays, to be read. An agent
                // stays either way: Enter runs it again in its session.
                let kind = self
                    .views
                    .iter()
                    .find_map(|v| v.slot(id))
                    .map(|s| s.kind.clone());
                let agent = matches!(kind, Some(Kind::Agent(..)));
                let clean = exit.code == 0 && exit.signal.is_none();
                if clean && let Some(Kind::Install { addons, root, name }) = &kind {
                    self.installed(addons, root.as_deref(), name);
                }
                if clean && !agent {
                    self.close(id);
                } else if let Some(slot) = self.slot_mut(id) {
                    slot.pane.exited(exit);
                    let session = slot.pane.session_id();
                    let _ = self.sessions.close(session);
                }
                if agent {
                    self.refresh_panel();
                }
            }
            Msg::Local(detection) => {
                self.services.set_local(detection);
                self.say("Looked for local models.");
                self.refresh_list();
            }
            Msg::FilesChanged(root) => {
                for view in &mut self.views {
                    if view.root() == Some(root.as_path())
                        && let Some(files) = view.files.as_mut()
                    {
                        files.reload();
                    }
                }
            }
        }
    }

    /// Gives every pane on screen its size in the layout. Sizes that did not
    /// change cost nothing.
    fn fit(&mut self) {
        if self.screen != Screen::Space {
            return;
        }
        let (Some(at), Some(layout)) = (self.view_at(), self.space_layout()) else {
            return;
        };
        let view = &mut self.views[at];
        for area in &layout.panes.panes {
            if let Some(slot) = view.slot_mut(area.id) {
                slot.pane.resize(TerminalSize {
                    cols: area.body.width,
                    rows: area.body.height,
                });
            }
        }
    }

    /// The soonest a pane's synchronized update must be shown.
    fn sync_deadline(&self) -> Option<Instant> {
        self.views
            .iter()
            .flat_map(|v| &v.slots)
            .filter_map(|s| s.pane.sync_deadline())
            .min()
    }

    fn end_due_syncs(&mut self) {
        let now = Instant::now();
        for slot in self.views.iter_mut().flat_map(|v| &mut v.slots) {
            if slot.pane.sync_deadline().is_some_and(|due| due <= now) {
                slot.pane.end_sync();
            }
        }
    }

    /// The cursor the user's terminal should show: the focused program's own,
    /// in a space.
    fn wanted_cursor(&self) -> Option<(CursorShape, bool)> {
        if self.screen != Screen::Space
            || self.question.is_some()
            || self.dialog.is_some()
            || !matches!(self.mode, Mode::Normal | Mode::Prefix)
        {
            return None;
        }
        let view = self.view()?;
        if view.focus != Focus::Panes {
            return None;
        }
        view.focused_slot().and_then(|s| s.pane.cursor_style())
    }

    /// Shows the program's cursor shape (a bar in vim's insert mode, say), and
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

/// The cell of `body` at `cell`, from its top left, kept inside it.
fn inside(body: Rect, cell: Position) -> (u16, u16) {
    let col = cell.x.clamp(body.x, body.right().saturating_sub(1)) - body.x;
    let row = cell.y.clamp(body.y, body.bottom().saturating_sub(1)) - body.y;
    (col, row)
}

/// Reads keys and the mouse on a thread of its own, so the loop can wait for
/// them and for output together.
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
        app.fit();
        app.refresh_states();
        terminal.draw(|frame: &mut Frame<'_>| ui::draw(frame, app))?;
        app.apply_cursor(terminal)?;
        if app.quit {
            return Ok(());
        }
        // Agents hung up from outside report no exit: look for it now and then.
        let settle = app
            .any_stopping()
            .then(|| Instant::now() + Duration::from_millis(100));
        let first = match app.sync_deadline().into_iter().chain(settle).min() {
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
        app.settle_stopped();
    }
}
