//! A space as `x8ai` shows it: its tabs, each a tree of panes (shells, or the
//! user's editor on a file), its file list, and which of them has the keys.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;
use x8ai_agents::AgentSession;
use x8ai_core::agent::AgentSessionId;
use x8ai_core::terminal::TerminalSize;
use x8ai_workspace::{Watcher, Workspace};

use crate::agents::{AgentRow, Isolated};
use crate::app::Msg;
use crate::files::FileList;
use crate::layout::{Axis, Layout, PaneArea, Tree};
use crate::listing::Listing;
use crate::pane::{Pane, PaneId};
use crate::spaces::SpaceInfo;

/// Longest tab label, in columns.
const MAX_LABEL: usize = 24;

/// The narrowest window that shows a sidebar beside the panes.
pub const MIN_WIDTH_FOR_SIDEBAR: u16 = 60;

/// What runs in a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Shell,
    /// The user's editor (`$VISUAL`, `$EDITOR`) on this file.
    Editor(PathBuf),
    /// An agent session's agent, by name.
    Agent(AgentSessionId, String),
    /// A pager on what a session's agent changed.
    Review(AgentSessionId, String),
    /// An add-on install: the add-ons it adds to the space of `root` when it
    /// ends well and they are found.
    Install {
        addons: Vec<&'static str>,
        root: Option<PathBuf>,
        name: String,
    },
}

pub struct Slot {
    pub pane: Pane,
    pub kind: Kind,
}

impl Slot {
    /// Whether closing it would end something running. A shell at its prompt
    /// is not; anything else that has not ended is.
    pub fn is_busy(&self) -> bool {
        match self.kind {
            Kind::Shell => self.pane.is_busy(),
            _ => self.pane.exit().is_none(),
        }
    }

    /// An editor by its file; a shell by the title its program set, or its name.
    pub fn title(&self) -> String {
        let title = match &self.kind {
            Kind::Agent(_, name) => name.clone(),
            Kind::Review(_, name) => format!("changes: {name}"),
            Kind::Install { name, .. } => format!("install: {name}"),
            Kind::Editor(path) => path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
            Kind::Shell => match self.pane.title().filter(|t| !t.trim().is_empty()) {
                Some(title) => title.to_owned(),
                // The shell's own name: zsh, bash, fish.
                None => Path::new(self.pane.program())
                    .file_name()
                    .map_or_else(|| "shell".to_owned(), |n| n.to_string_lossy().into_owned()),
            },
        };
        clip(
            &title
                .chars()
                .filter(|c| !c.is_control())
                .collect::<String>(),
            MAX_LABEL,
        )
    }
}

pub struct Tab {
    pub tree: Tree,
    pub focused: PaneId,
    /// The focused pane alone fills the tab.
    pub zoomed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Panes,
    Sidebar,
}

/// What the sidebar beside the panes shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sidebar {
    Hidden,
    Files,
    Agents,
    Models,
    Mcp,
    Catalog,
    Addons,
}

impl Sidebar {
    /// The panels that show a [`Listing`] (`list`).
    pub fn is_list(self) -> bool {
        matches!(
            self,
            Self::Models | Self::Mcp | Self::Catalog | Self::Addons
        )
    }
}

/// What a row of a list panel is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListKey {
    Provider(String),
    /// A provider's model, and whether the user added it.
    Model {
        provider: String,
        model: String,
        added: bool,
    },
    Mcp(String),
    /// A catalog item, by its catalog id (`model.anthropic.claude-sonnet-5`).
    Catalog(String),
    Addon(&'static str),
}

/// A row of the Agents panel: an agent to launch, or one of the space's
/// agent sessions.
#[derive(Debug, Clone)]
pub enum PanelRow {
    Agent(Box<AgentRow>),
    Session(Box<AgentSession>),
}

/// The Agents panel's rows and selection, read again when it opens and when a
/// session changes.
#[derive(Default)]
pub struct AgentsPanel {
    pub rows: Vec<PanelRow>,
    pub selected: usize,
    pub offset: usize,
    pub isolated: Option<Isolated>,
    /// What each agent's next launch gets, in a few words, by agent id.
    pub drafts: std::collections::HashMap<String, String>,
}

/// A line of the Agents panel, top to bottom.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelLine {
    /// Whether the folder is trusted.
    Trust,
    /// How agents work here: worktrees, or directly in the folder.
    Isolation,
    Blank,
    Heading(&'static str),
    /// Row `n` of `rows`, which can be selected.
    Row(usize),
    /// What agent row `n`'s next launch gets, under it.
    Draft(usize),
    /// Said when a section is empty.
    Empty(&'static str),
}

impl AgentsPanel {
    pub fn move_by(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    pub fn row(&self) -> Option<&PanelRow> {
        self.rows.get(self.selected)
    }

    /// Every line, in order: the folder's trust and isolation, the agents,
    /// then the sessions.
    pub fn lines(&self) -> Vec<PanelLine> {
        let mut lines = vec![PanelLine::Trust, PanelLine::Isolation, PanelLine::Blank];
        lines.push(PanelLine::Heading("NEW SESSION"));
        let agents = self
            .rows
            .iter()
            .filter(|r| matches!(r, PanelRow::Agent(_)))
            .count();
        if agents == 0 {
            lines.push(PanelLine::Empty("No agents."));
        }
        for row in 0..agents {
            lines.push(PanelLine::Row(row));
            if let Some(PanelRow::Agent(agent)) = self.rows.get(row)
                && self.drafts.contains_key(agent.definition.id.as_str())
            {
                lines.push(PanelLine::Draft(row));
            }
        }
        lines.push(PanelLine::Blank);
        lines.push(PanelLine::Heading("SESSIONS"));
        if agents == self.rows.len() {
            lines.push(PanelLine::Empty("None yet: Enter on an agent starts one."));
        }
        lines.extend((agents..self.rows.len()).map(PanelLine::Row));
        lines
    }

    /// The row at line `line` of the panel (from its first line), if any.
    pub fn row_at(&self, line: usize) -> Option<usize> {
        match self.lines().get(self.offset + line) {
            Some(PanelLine::Row(row)) => Some(*row),
            _ => None,
        }
    }

    /// Scrolls so the selection shows in `height` lines.
    pub fn keep_in_view(&mut self, height: usize) {
        let lines = self.lines();
        let Some(at) = lines
            .iter()
            .position(|l| *l == PanelLine::Row(self.selected))
        else {
            self.offset = 0;
            return;
        };
        if height == 0 {
            return;
        }
        // The headings above the first rows show when it is selected.
        let top = if self.selected == 0 { 0 } else { at };
        if top < self.offset {
            self.offset = top;
        } else if at >= self.offset + height {
            self.offset = at + 1 - height;
        }
    }
}

/// A label in the header: a tab, or `+` for a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Label {
    Tab(usize),
    New,
}

/// Where everything in a space is on the screen.
pub struct SpaceLayout {
    pub header: Rect,
    pub labels: Vec<(Label, Rect)>,
    /// The sidebar (the file list or the Agents panel), its right border
    /// included.
    pub sidebar: Option<Rect>,
    pub panes: Layout,
    pub status: Rect,
}

impl SpaceLayout {
    /// The rows the sidebar's entries take (below its title row).
    pub fn sidebar_rows(&self) -> Option<Rect> {
        self.sidebar.map(|f| Rect {
            y: f.y + 1,
            height: f.height.saturating_sub(1),
            width: f.width.saturating_sub(1),
            ..f
        })
    }
}

pub struct SpaceView {
    pub info: SpaceInfo,
    /// Where it is, as shown (`~/code/app`).
    pub place: String,
    pub tabs: Vec<Tab>,
    pub active: usize,
    pub slots: Vec<Slot>,
    /// `None` for the workspace with no folder, or a folder that cannot be read.
    pub files: Option<FileList>,
    pub sidebar: Sidebar,
    pub panel: AgentsPanel,
    /// The list of the Models, MCP, Catalog or Add-ons panel, whichever shows.
    pub list: Listing<ListKey>,
    pub focus: Focus,
    _watcher: Option<Watcher>,
}

impl SpaceView {
    /// The space's view, with its file list followed on disk: a change sends
    /// `FilesChanged` on `tx`.
    pub fn new(info: SpaceInfo, place: String, tx: &Sender<Msg>) -> Self {
        let mut watcher = None;
        let files = info.root.as_deref().and_then(|root| {
            let workspace = Workspace::reopen(root).ok()?;
            let changed = root.to_owned();
            let tx = tx.clone();
            watcher = workspace
                .watch(move |_| drop(tx.send(Msg::FilesChanged(changed.clone()))))
                .ok();
            Some(FileList::new(workspace))
        });
        Self {
            info,
            place,
            tabs: Vec::new(),
            active: 0,
            slots: Vec::new(),
            files,
            sidebar: Sidebar::Hidden,
            panel: AgentsPanel::default(),
            list: Listing::default(),
            focus: Focus::Panes,
            _watcher: watcher,
        }
    }

    pub fn root(&self) -> Option<&Path> {
        self.info.root.as_deref()
    }

    pub fn tab(&self) -> Option<&Tab> {
        self.tabs.get(self.active)
    }

    pub fn tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.active)
    }

    /// The pane that has the keys in the open tab.
    pub fn focused(&self) -> Option<PaneId> {
        self.tab().map(|t| t.focused)
    }

    pub fn slot(&self, id: PaneId) -> Option<&Slot> {
        self.slots.iter().find(|s| s.pane.id == id)
    }

    pub fn slot_mut(&mut self, id: PaneId) -> Option<&mut Slot> {
        self.slots.iter_mut().find(|s| s.pane.id == id)
    }

    pub fn focused_slot(&self) -> Option<&Slot> {
        self.focused().and_then(|id| self.slot(id))
    }

    pub fn focused_slot_mut(&mut self) -> Option<&mut Slot> {
        let id = self.focused()?;
        self.slot_mut(id)
    }

    /// A new tab with `slot` in it, shown.
    pub fn add_tab(&mut self, slot: Slot) {
        let id = slot.pane.id;
        self.slots.push(slot);
        self.tabs.push(Tab {
            tree: Tree::Pane(id),
            focused: id,
            zoomed: false,
        });
        self.active = self.tabs.len() - 1;
        self.focus = Focus::Panes;
    }

    /// Splits the focused pane: `slot` goes right of it or below it, focused.
    pub fn split(&mut self, slot: Slot, axis: Axis, split: u32) {
        let id = slot.pane.id;
        let Some(tab) = self.tabs.get_mut(self.active) else {
            self.add_tab(slot);
            return;
        };
        tab.tree.split(tab.focused, id, axis, split);
        tab.focused = id;
        tab.zoomed = false;
        self.slots.push(slot);
        self.focus = Focus::Panes;
    }

    /// Takes pane `id` out of its tab, which goes when it was its last pane.
    /// The slot is handed back to be closed.
    pub fn remove(&mut self, id: PaneId) -> Option<Slot> {
        let at = self.slots.iter().position(|s| s.pane.id == id)?;
        let slot = self.slots.remove(at);
        if let Some(t) = self.tabs.iter().position(|t| t.tree.contains(id)) {
            let tab = &mut self.tabs[t];
            if tab.tree.remove(id) {
                if tab.focused == id {
                    tab.focused = tab.tree.panes()[0];
                }
                tab.zoomed = false;
            } else {
                self.tabs.remove(t);
                if self.active > t || self.active == self.tabs.len() {
                    self.active = self.active.saturating_sub(1);
                }
            }
        }
        Some(slot)
    }

    /// Puts `slot` where pane `id` is, focused if `id` was. The old slot is
    /// handed back to be closed.
    pub fn replace(&mut self, id: PaneId, slot: Slot) -> Option<Slot> {
        let at = self.slots.iter().position(|s| s.pane.id == id)?;
        let added = slot.pane.id;
        for tab in &mut self.tabs {
            if tab.tree.replace(id, added) && tab.focused == id {
                tab.focused = added;
            }
        }
        self.slots.push(slot);
        Some(self.slots.remove(at))
    }

    /// The tab whose editor has `path` open, if its editor is still running.
    pub fn editor_tab(&self, path: &Path) -> Option<usize> {
        self.tabs.iter().position(|tab| {
            tab.tree.panes().iter().any(|&id| {
                self.slot(id).is_some_and(|s| {
                    s.kind == Kind::Editor(path.to_owned()) && s.pane.exit().is_none()
                })
            })
        })
    }

    /// Shows tab `index`, if there is one.
    pub fn show_tab(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
            self.focus = Focus::Panes;
        }
    }

    /// A tab's label: its number and its focused pane's title.
    pub fn label(&self, index: usize) -> String {
        let title = self
            .slot(self.tabs[index].focused)
            .map_or_else(|| "…".to_owned(), Slot::title);
        format!(" {} {title} ", index + 1)
    }

    /// The header's right end: where the space is, and whether it is trusted.
    /// At most `width` columns: a long path loses its start, never the trust.
    /// The tab bar's right end: the space's folder (its trust is in the
    /// status bar, as in the app).
    pub fn right_text(&self, width: usize) -> String {
        if self.info.root.is_none() {
            return "no folder ".to_owned();
        }
        format!("{} ", clip_start(&self.place, width.saturating_sub(2)))
    }

    /// Places the header, the file list, the open tab's panes and the status
    /// bar in `area`.
    pub fn layout(&self, area: Rect) -> SpaceLayout {
        let header = Rect { height: 1, ..area };
        let status = Rect {
            y: area.bottom().saturating_sub(1),
            height: 1.min(area.height),
            ..area
        };
        let body = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(2),
            ..area
        };

        // The header: " x8ai <name>  " then the tabs, then `+`.
        let mut labels = Vec::new();
        let mut x = area.x + u16::try_from(6 + self.info.name.width() + 2).unwrap_or(u16::MAX);
        let right =
            u16::try_from(self.right_text(usize::from(area.width / 3)).width()).unwrap_or(u16::MAX);
        let end = area.right().saturating_sub(right + 1);
        let mut place = |label: Label, width: usize| {
            let width = u16::try_from(width).unwrap_or(u16::MAX);
            if x.saturating_add(width) <= end {
                labels.push((label, Rect::new(x, header.y, width, 1)));
                x += width + 1;
                true
            } else {
                false
            }
        };
        for index in 0..self.tabs.len() {
            if !place(Label::Tab(index), self.label(index).width()) {
                break;
            }
        }
        place(Label::New, 3);

        let (sidebar, panes_area) = self.split_body(body);
        let panes = match self.tab() {
            Some(tab) if tab.zoomed => Tree::Pane(tab.focused).layout(panes_area),
            Some(tab) => tab.tree.layout(panes_area),
            None => Layout::default(),
        };
        SpaceLayout {
            header,
            labels,
            sidebar,
            panes,
            status,
        }
    }

    /// The body (between the header and the status bar) divided between the
    /// file list, when shown, and the panes.
    fn split_body(&self, body: Rect) -> (Option<Rect>, Rect) {
        let width = match self.sidebar {
            Sidebar::Files if self.files.is_some() => Some((body.width / 4).clamp(18, 36)),
            Sidebar::Agents
            | Sidebar::Models
            | Sidebar::Mcp
            | Sidebar::Catalog
            | Sidebar::Addons => Some((body.width * 2 / 5).clamp(32, 56)),
            _ => None,
        };
        let sidebar = width
            .filter(|_| body.width >= MIN_WIDTH_FOR_SIDEBAR)
            .map(|width| Rect { width, ..body });
        let panes = match sidebar {
            Some(sidebar) => Rect {
                x: sidebar.right(),
                width: body.width - sidebar.width,
                ..body
            },
            None => body,
        };
        (sidebar, panes)
    }

    /// The size a new pane gets in the window's `area`: a tab of its own, or
    /// half the focused pane (`split`), less the title rows splitting adds.
    pub fn new_pane_size(&self, area: Rect, split: Option<Axis>) -> TerminalSize {
        let body = Rect {
            y: area.y + 1,
            height: area.height.saturating_sub(2),
            ..area
        };
        let whole = self.split_body(body).1;
        let layout = self.layout(area);
        let focused = self.focused().and_then(|id| layout.panes.area_of(id));
        let (cols, rows) = match (split, focused.map(PaneArea::whole)) {
            (Some(Axis::Right), Some(pane)) => (
                pane.width.saturating_sub(1) / 2,
                pane.height.saturating_sub(1),
            ),
            (Some(Axis::Down), Some(pane)) => (pane.width, (pane.height / 2).saturating_sub(1)),
            _ => (whole.width, whole.height),
        };
        TerminalSize {
            cols: cols.max(1),
            rows: rows.max(1),
        }
    }

    /// The panes whose closing would end something running: a program a
    /// shell started, or an editor, agent or pager that has not ended.
    pub fn busy(&self) -> impl Iterator<Item = &Slot> {
        self.slots.iter().filter(|s| s.is_busy())
    }

    /// The pane running session `id`'s agent, if any.
    pub fn agent_pane(&self, id: AgentSessionId) -> Option<PaneId> {
        self.slots
            .iter()
            .find(|s| matches!(&s.kind, Kind::Agent(session, _) if *session == id))
            .map(|s| s.pane.id)
    }

    /// Shows the tab holding pane `id`, focused.
    pub fn show_pane(&mut self, id: PaneId) {
        if let Some(index) = self.tabs.iter().position(|t| t.tree.contains(id)) {
            self.active = index;
            self.tabs[index].focused = id;
            self.focus = Focus::Panes;
        }
    }
}

/// `text` cut to `width` columns, with `…` when cut.
pub fn clip(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    for c in text.chars() {
        if out.width() + c.to_string().width() + 1 > width {
            break;
        }
        out.push(c);
    }
    out.push('…');
    out
}

/// `text` cut to `width` columns from its start, with `…` when cut: the end
/// of a path says most about it.
pub fn clip_start(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut kept: Vec<char> = Vec::new();
    let mut used = 1;
    for c in text.chars().rev() {
        let w = c.to_string().width();
        if used + w > width {
            break;
        }
        used += w;
        kept.push(c);
    }
    std::iter::once('…').chain(kept.into_iter().rev()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_paths_keep_their_end() {
        assert_eq!(clip_start("~/code/app", 20), "~/code/app");
        assert_eq!(
            clip_start("/private/tmp/a/very/long/repo", 10),
            "…long/repo"
        );
    }

    #[test]
    fn long_titles_are_clipped() {
        assert_eq!(clip("main.rs", 10), "main.rs");
        assert_eq!(clip("a-very-long-file-name.rs", 10), "a-very-lo…");
        assert_eq!(clip("日本語のファイル", 7), "日本語…");
    }
}
