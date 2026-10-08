//! The Agents panel (Ctrl-g a) and what it starts: agents launched in their
//! own sessions after the folder is trusted and the agent allowed there, as the
//! app's dialogs ask (docs/agent-runtime.md); sessions run again, stopped,
//! reviewed and removed (docs/multi-agent.md).
//!
//! Every question is answered in `x8ai` itself, by the user at their keyboard:
//! nothing a program in a pane prints can answer it, since its output never
//! reaches `x8ai`'s keys. Trust and approval are checked again, natively, just
//! before an agent runs (`Spaces::authorize`).

use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use ratatui::crossterm::event::{KeyCode, KeyEvent};
use x8ai_agents::{AgentSession, Denied, LaunchPlan, SessionState};
use x8ai_core::agent::AgentSessionId;
use x8ai_pty::{Environment, Program};

use super::{App, Ask, Question, Then};
use crate::agents::{Isolated, runs_here};
use crate::pane::{Pane, PaneId};
use crate::space::{Focus, Kind, PanelRow, Sidebar, Slot};
use crate::welcome::tilde;

/// Which row stays selected when the panel is read again.
#[derive(PartialEq, Eq)]
enum Selected {
    Agent(String),
    Session(AgentSessionId),
}

impl App {
    /// Ctrl-g a: the Agents panel, with the keys; hidden when it has them.
    pub(super) fn toggle_agents(&mut self) {
        let Some(at) = self.view_at() else {
            return;
        };
        if self.current.root.is_none() {
            self.say(
                "Agents work in a folder: open one with /cd first (ctrl-g h for the Welcome).",
            );
            return;
        }
        self.toggle_sidebar(at, Sidebar::Agents);
        self.refresh_panel();
    }

    /// Reads the open space's agents and sessions again, while the panel shows.
    pub(super) fn refresh_panel(&mut self) {
        let Some(at) = self.view_at() else {
            return;
        };
        let Some(root) = self.views[at].root().map(Path::to_owned) else {
            return;
        };
        if self.views[at].sidebar != Sidebar::Agents {
            return;
        }
        let spaces = &mut self.spaces;
        let agents = &self.agents;
        let mut rows: Vec<PanelRow> = agents
            .rows(&root, |plan| spaces.is_approved_for_any_provider(plan))
            .into_iter()
            .map(|agent| PanelRow::Agent(Box::new(agent)))
            .collect();
        {
            rows.extend(
                agents
                    .sessions_in(&root)
                    .into_iter()
                    .map(|s| PanelRow::Session(Box::new(s))),
            );
        }
        let trusted = spaces.is_trusted(&root);
        let isolated = Some(agents.isolation_of(&root));

        self.current.trusted = trusted;
        let view = &mut self.views[at];
        view.info.trusted = trusted;
        let panel = &mut view.panel;
        let was = panel.row().map(selected);
        panel.selected = was
            .and_then(|was| rows.iter().position(|r| selected(r) == was))
            .unwrap_or(panel.selected)
            .min(rows.len().saturating_sub(1));
        panel.rows = rows;
        panel.isolated = isolated;
    }

    /// Keys for the Agents panel.
    pub(super) fn agents_key(&mut self, key: KeyEvent) {
        let (Some(at), Some(layout)) = (self.view_at(), self.space_layout()) else {
            return;
        };
        let height = layout
            .sidebar_rows()
            .map_or(1, |r| usize::from(r.height).max(1));
        let row = self.views[at].panel.row().cloned();
        let session = match &row {
            Some(PanelRow::Session(session)) => Some((**session).clone()),
            _ => None,
        };
        let panel = &mut self.views[at].panel;
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => panel.move_by(-1),
            KeyCode::Down | KeyCode::Char('j') => panel.move_by(1),
            KeyCode::Home | KeyCode::Char('g') => panel.selected = 0,
            KeyCode::End | KeyCode::Char('G') => panel.move_by(isize::MAX),
            KeyCode::Esc | KeyCode::Tab => self.views[at].focus = Focus::Panes,
            KeyCode::Char('t') => self.ask_trust_or_untrust(),
            KeyCode::Enter => match row {
                Some(PanelRow::Agent(agent)) => self.launch(agent.definition.id.as_str()),
                Some(PanelRow::Session(session)) => self.open_session(session.id),
                None => {}
            },
            KeyCode::Char('r') => {
                if let Some(PanelRow::Agent(agent)) = row {
                    self.revoke(agent.definition.id.as_str(), &agent.definition.name);
                }
            }
            KeyCode::Char('c') => {
                if let Some(session) = session {
                    self.review(&session);
                }
            }
            KeyCode::Char('o') => {
                if let Some(session) = session {
                    self.open_shell_in(&session);
                }
            }
            KeyCode::Char('s') => {
                if let Some(session) = session {
                    self.stop_session(&session);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                if let Some(session) = session {
                    self.ask_remove(&session);
                }
            }
            _ => {}
        }
        if let Some(view) = self.views.get_mut(at) {
            view.panel.keep_in_view(height);
        }
    }

    /// A click on line `line` of the panel's rows selects what is there.
    pub(super) fn click_panel_row(&mut self, at: usize, line: usize) {
        let panel = &mut self.views[at].panel;
        if let Some(row) = panel.row_at(line) {
            panel.selected = row;
        }
    }

    // Launching and running

    /// Enter on an agent: a new session for it, after asking to trust the
    /// folder and to allow the agent there, when needed.
    fn launch(&mut self, agent: &str) {
        let Some(root) = self.current.root.clone() else {
            return;
        };
        let plan = match self.agents.plan(agent, &root) {
            Ok(plan) => plan,
            Err(error) => {
                self.complain(error);
                return;
            }
        };
        let then = Then::Launch(agent.to_owned());
        if !self.spaces.is_trusted(&root) {
            self.ask_trust(root, Some(then));
            return;
        }
        if !self.spaces.is_approved(&plan) {
            self.ask_approve(plan, then);
            return;
        }
        if let Err(denied) = self.spaces.authorize(&plan) {
            self.complain(denied.to_string());
            return;
        }
        match self.agents.create(&plan) {
            Ok(session) => {
                self.refresh_panel();
                self.run_session(session, None);
            }
            Err(error) => self.complain(format!("Could not start {}: {error}", plan.name)),
        }
    }

    /// Enter on a session: its agent's pane, or its agent run again.
    fn open_session(&mut self, id: AgentSessionId) {
        let Some(at) = self.view_at() else {
            return;
        };
        match self.views[at].agent_pane(id) {
            Some(pane) => self.views[at].show_pane(pane),
            None => self.run_session(id, None),
        }
    }

    /// Runs session `id`'s agent on a new pane, in a tab of its own or in place
    /// of pane `replace` (its ended agent). Trust and approval are checked
    /// again first, and asked for if they are missing.
    pub(super) fn run_session(&mut self, id: AgentSessionId, replace: Option<PaneId>) {
        let Some(at) = self.view_at() else {
            return;
        };
        let Some(record) = self.agents.runtime.get(id) else {
            self.complain("That session is gone.");
            return;
        };
        if !runs_here(&record) {
            self.complain(format!(
                "This {} session was made in the app with a model, MCP servers or skills: run it from the app for now. They come to x8ai in the next step.",
                record.name
            ));
            return;
        }
        if self.current.root.as_deref() != Some(record.workspace.as_path()) {
            self.complain("That session belongs to another folder.");
            return;
        }
        let plan = match self.agents.plan(record.agent.as_str(), &record.workspace) {
            Ok(plan) => plan,
            Err(error) => {
                self.agents.runtime.fail(id, error.clone());
                self.complain(error);
                return;
            }
        };
        let authorized = match self.spaces.authorize(&plan) {
            Ok(authorized) => authorized,
            Err(Denied::Untrusted(root)) => {
                self.ask_trust(root, Some(Then::Run(id)));
                return;
            }
            Err(Denied::NotApproved { .. }) => {
                self.ask_approve(plan.clone(), Then::Run(id));
                return;
            }
            Err(other) => {
                self.agents.runtime.fail(id, other.to_string());
                self.complain(other.to_string());
                return;
            }
        };
        let size = replace
            .and_then(|old| {
                let layout = self.space_layout()?;
                let body = layout.panes.area_of(old)?.body;
                Some(x8ai_core::terminal::TerminalSize {
                    cols: body.width.max(1),
                    rows: body.height.max(1),
                })
            })
            .unwrap_or_else(|| self.new_pane_size(at, None));
        let pane = PaneId(self.next_pane);
        self.next_pane += 1;
        let events = crate::pane::events(pane, self.tx.clone());
        match self
            .agents
            .runtime
            .run(&self.sessions, id, authorized, size, events)
        {
            Ok(session) => {
                let slot = Slot {
                    pane: Pane::new(pane, session, size),
                    kind: Kind::Agent(id, record.name.clone()),
                };
                let view = &mut self.views[at];
                match replace.filter(|&old| view.slot(old).is_some()) {
                    Some(old) => {
                        if let Some(ended) = view.replace(old, slot) {
                            let _ = self.sessions.close(ended.pane.session_id());
                        }
                        view.show_pane(pane);
                    }
                    None => view.add_tab(slot),
                }
            }
            Err(error) => self.complain(format!("{} could not start: {error}", record.name)),
        }
        self.refresh_panel();
    }

    // Questions

    fn ask_trust_or_untrust(&mut self) {
        let Some(root) = self.current.root.clone() else {
            return;
        };
        if self.spaces.is_trusted(&root) {
            self.question = Some(Question {
                title: format!("Stop trusting “{}”?", self.current.name),
                lines: vec![
                    "Its agents stop, and the approvals given here are removed. Its sessions and worktrees stay.".to_owned(),
                ],
                yes: "stop trusting",
                ask: Ask::Untrust(root),
            });
        } else {
            self.ask_trust(root, None);
        }
    }

    fn ask_trust(&mut self, root: PathBuf, then: Option<Then>) {
        self.question = Some(Question {
            title: format!("Trust “{}”?", self.current.name),
            lines: vec![
                tilde(&root, self.spaces.home()),
                "Agents run only in folders you trust. Trust a folder only if you trust the code in it: a repository can hold files that try to steer an agent.".to_owned(),
                "You can stop trusting it at any time: t in the Agents panel.".to_owned(),
            ],
            yes: "trust",
            ask: Ask::Trust(root, then),
        });
    }

    fn ask_approve(&mut self, plan: LaunchPlan, then: Then) {
        let name = plan.name.clone();
        let folder = tilde(&plan.workspace, self.spaces.home());
        let command = std::iter::once(plan.program.display().to_string())
            .chain(plan.args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        let place = match self.agents.isolation_of(&plan.workspace) {
            Isolated::Worktrees { .. } => {
                "Each session works in a Git worktree of its own, on its own branch: your working tree and branch are not touched.".to_owned()
            }
            Isolated::Shared(reason) => {
                format!("Not isolated ({reason}): it works directly in your files, one agent at a time.")
            }
        };
        self.question = Some(Question {
            title: format!("Allow {name} to work in “{}”?", self.current.name),
            lines: vec![
                format!("Folder: {folder}"),
                format!("Program: {command}"),
                format!("Model: {name}'s own configuration (its settings and your shell)."),
                place,
                format!(
                    "{name} runs as you, with access to your files, network and credentials, as if you started it in a terminal yourself. This allows it in this folder only: r in the Agents panel takes it back."
                ),
            ],
            yes: "allow",
            ask: Ask::Approve(Box::new(plan), then),
        });
    }

    /// y to "Trust …?": trusts the folder, then goes on.
    pub(super) fn trust(&mut self, root: &Path, then: Option<Then>) {
        if let Err(error) = self.spaces.set_trust(root, true) {
            self.complain(format!("Could not trust it: {error}"));
            return;
        }
        self.refresh_trust(root);
        self.go_on(then);
    }

    /// y to "Stop trusting …?": approvals go and agents stop.
    pub(super) fn untrust(&mut self, root: &Path) {
        if let Err(error) = self.spaces.set_trust(root, false) {
            self.complain(format!("Could not stop trusting it: {error}"));
            return;
        }
        let stopped = self.agents.runtime.stop_in(&self.sessions, root);
        let here: Vec<AgentSessionId> = self
            .agents
            .runtime
            .sessions_in(root)
            .iter()
            .map(|s| s.id)
            .collect();
        self.mark_stopping(|kind| matches!(kind, Kind::Agent(id, _) if here.contains(id)));
        self.refresh_trust(root);
        self.say(match stopped {
            0 => {
                "No longer trusted: agents cannot run here, and its approvals are gone.".to_owned()
            }
            n => format!(
                "No longer trusted: {n} running agent(s) stopped, and its approvals are gone."
            ),
        });
    }

    /// y to "Allow …?": records the approval, if the folder is still the one
    /// open and still trusted, then goes on.
    pub(super) fn approve(&mut self, plan: &LaunchPlan, then: Then) {
        let root = &plan.workspace;
        if self.current.root.as_deref() != Some(root.as_path()) || !self.spaces.is_trusted(root) {
            self.complain("The folder changed while asking; nothing was allowed.");
            return;
        }
        if let Err(error) = self.spaces.approve(plan) {
            self.complain(format!("Could not allow it: {error}"));
            return;
        }
        self.go_on(Some(then));
    }

    fn go_on(&mut self, then: Option<Then>) {
        match then {
            Some(Then::Launch(agent)) => self.launch(&agent),
            Some(Then::Run(id)) => {
                let ended = self.view().and_then(|v| v.agent_pane(id)).filter(|&pane| {
                    self.view()
                        .and_then(|v| v.slot(pane))
                        .is_some_and(|s| s.pane.exit().is_some())
                });
                self.run_session(id, ended);
            }
            None => self.refresh_panel(),
        }
    }

    /// After trust changed: the space as shown, and the panel.
    fn refresh_trust(&mut self, root: &Path) {
        let trusted = self.spaces.is_trusted(root);
        for view in &mut self.views {
            if view.root() == Some(root) {
                view.info.trusted = trusted;
            }
        }
        if self.current.root.as_deref() == Some(root) {
            self.current.trusted = trusted;
        }
        self.refresh_panel();
    }

    fn revoke(&mut self, agent: &str, name: &str) {
        let Some(root) = self.current.root.clone() else {
            return;
        };
        match self.spaces.revoke(&root, agent) {
            Ok(()) => self.say(format!(
                "{name} is no longer allowed here; it asks again next time."
            )),
            Err(error) => self.complain(format!("Could not take it back: {error}")),
        }
        self.refresh_panel();
    }

    // Sessions

    fn stop_session(&mut self, session: &AgentSession) {
        if session.state != SessionState::Running {
            self.say(format!("{} is not running.", session.name));
            return;
        }
        match self.agents.runtime.stop(&self.sessions, session.id) {
            Ok(()) => {
                let id = session.id;
                self.mark_stopping(|kind| matches!(kind, Kind::Agent(agent, _) if *agent == id));
                self.say(format!(
                    "Stopping {}. Its session and worktree stay.",
                    session.name
                ));
            }
            Err(error) => self.complain(error.to_string()),
        }
    }

    /// c: what the agent changed, in a pager, in a tab of its own.
    fn review(&mut self, session: &AgentSession) {
        let Some(at) = self.view_at() else {
            return;
        };
        let changes = match self.agents.changes(session) {
            Ok(changes) => changes,
            Err(error) => {
                self.complain(error);
                return;
            }
        };
        let text = crate::agents::review(&session.name, &changes);
        let file = std::env::temp_dir().join(format!(
            "x8ai-review-{}-{}.txt",
            std::process::id(),
            session.id.0
        ));
        if let Err(error) = write_private(&file, text.as_bytes()) {
            self.complain(format!("Could not write the review: {error}"));
            return;
        }
        // The user's LESS could make less quit at once on a short review.
        let env = self
            .agents
            .env()
            .iter()
            .filter(|(name, _)| name != "LESS")
            .cloned()
            .collect();
        let program = Program::Exec {
            program: self.agents.program("less", "/usr/bin/less"),
            args: vec!["-R".into(), "-+F".into(), file.into_os_string()],
            cwd: Some(session.cwd.clone()),
            env: Environment::Exactly(env),
        };
        let size = self.new_pane_size(at, None);
        match self.start(
            &program,
            Kind::Review(session.id, session.name.clone()),
            size,
        ) {
            Ok(slot) => self.views[at].add_tab(slot),
            Err(error) => self.complain(format!("Could not show the changes: {error}")),
        }
    }

    /// o: a shell in the session's workspace, to look around or use Git there.
    fn open_shell_in(&mut self, session: &AgentSession) {
        let Some(at) = self.view_at() else {
            return;
        };
        let program = Program::LoginShell {
            cwd: Some(session.cwd.clone()),
            env: self
                .current
                .id
                .iter()
                .map(|id| ("X8AI_SPACE".to_owned(), id.clone()))
                .collect(),
        };
        let size = self.new_pane_size(at, None);
        match self.start(&program, Kind::Shell, size) {
            Ok(slot) => self.views[at].add_tab(slot),
            Err(error) => self.complain(format!("Could not start a shell: {error}")),
        }
    }

    /// d: asks to remove a stopped session, saying what goes and what stays.
    fn ask_remove(&mut self, session: &AgentSession) {
        if session.state == SessionState::Running {
            self.complain(format!(
                "{} is running: stop it first (s here, or ctrl-g x on its pane).",
                session.name
            ));
            return;
        }
        let (lines, discard) = match &session.worktree {
            None => (
                vec!["It worked directly in your folder: nothing there is deleted.".to_owned()],
                false,
            ),
            Some(worktree) => {
                let changes = self.agents.changes(session).ok();
                let uncommitted = changes.as_ref().is_some_and(|c| c.uncommitted);
                let commits = changes.as_ref().map_or(0, |c| c.commits);
                let mut lines = vec![format!(
                    "Its workspace {} is deleted.",
                    tilde(&worktree.path, self.spaces.home())
                )];
                if uncommitted {
                    lines.push("Its changes that are not committed are discarded.".to_owned());
                }
                lines.push(if commits > 0 {
                    format!(
                        "Its branch {} keeps its {commits} commit(s): nothing committed is deleted.",
                        worktree.branch
                    )
                } else {
                    format!(
                        "Its branch {} has no commits, and goes too.",
                        worktree.branch
                    )
                });
                (lines, uncommitted)
            }
        };
        self.question = Some(Question {
            title: format!("Remove this {} session?", session.name),
            lines,
            yes: "remove",
            ask: Ask::Remove(session.id, discard),
        });
    }

    /// y to "Remove …?".
    pub(super) fn remove_session(&mut self, id: AgentSessionId, discard: bool) {
        let Some(session) = self.agents.runtime.get(id) else {
            return;
        };
        if session.state == SessionState::Running {
            self.complain(format!("{} is running: stop it first.", session.name));
            return;
        }
        match self.agents.remove(&session, discard) {
            Ok(kept) => {
                // Its ended agent's pane and its review go with it.
                let panes: Vec<PaneId> = self
                    .views
                    .iter()
                    .flat_map(|v| &v.slots)
                    .filter(|slot| {
                        matches!(&slot.kind, Kind::Agent(session, _) | Kind::Review(session, _) if *session == id)
                    })
                    .map(|s| s.pane.id)
                    .collect();
                for pane in panes {
                    self.close(pane);
                }
                match kept {
                    Some((branch, commits)) => self.say(format!(
                        "Removed. Its branch {branch} keeps its {commits} commit(s): `git merge {branch}` brings them into yours."
                    )),
                    None => self.say("Removed."),
                }
            }
            Err(error) => self.complain(format!("Could not remove it: {error}")),
        }
        self.refresh_panel();
    }
}

impl App {
    /// Marks the agent panes `which` picks as hung up from outside, so their
    /// exit is looked for.
    fn mark_stopping(&mut self, which: impl Fn(&Kind) -> bool) {
        for slot in self.views.iter_mut().flat_map(|v| &mut v.slots) {
            if which(&slot.kind) {
                slot.pane.mark_stopping();
            }
        }
    }

    /// Brings the panel's sessions up to date with the runtime (running,
    /// ended), without asking Git again. Cheap enough for every frame.
    pub(super) fn refresh_states(&mut self) {
        let Some(at) = self.view_at() else {
            return;
        };
        if self.views[at].sidebar != Sidebar::Agents {
            return;
        }
        for row in &mut self.views[at].panel.rows {
            if let PanelRow::Session(session) = row
                && let Some(now) = self.agents.runtime.get(session.id)
            {
                **session = now;
            }
        }
    }

    /// Whether a pane hung up from outside is still ending.
    pub(super) fn any_stopping(&self) -> bool {
        !self.ending.is_empty()
            || self
                .views
                .iter()
                .flat_map(|v| &v.slots)
                .any(|s| s.pane.is_stopping())
    }

    /// Records the exits of panes hung up from outside that have ended.
    pub(super) fn settle_stopped(&mut self) {
        let mut ended = false;
        for slot in self.views.iter_mut().flat_map(|v| &mut v.slots) {
            ended |= slot.pane.settle();
        }
        let runtime = &self.agents.runtime;
        let before = self.ending.len();
        self.ending.retain(|&id| {
            runtime
                .get(id)
                .is_some_and(|s| s.state == SessionState::Running)
        });
        ended |= self.ending.len() != before;
        if ended {
            self.refresh_panel();
        }
    }
}

/// The row a panel row is, to find it again when the panel is read again.
fn selected(row: &PanelRow) -> Selected {
    match row {
        PanelRow::Agent(agent) => Selected::Agent(agent.definition.id.as_str().to_owned()),
        PanelRow::Session(session) => Selected::Session(session.id),
    }
}

/// Writes `file` readable only by the user, replacing it.
fn write_private(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let _ = fs::remove_file(file);
    let mut out = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(file)?;
    out.write_all(bytes)
}
