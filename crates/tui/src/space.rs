//! A space as `x8ai` shows it: its tabs, each a tree of panes (shells, or the
//! user's editor on a file), its file list, and which of them has the keys.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use ratatui::layout::Rect;
use unicode_width::UnicodeWidthStr;
use x8ai_core::terminal::TerminalSize;
use x8ai_workspace::{Watcher, Workspace};

use crate::app::Msg;
use crate::files::FileList;
use crate::layout::{Axis, Layout, PaneArea, Tree};
use crate::pane::{Pane, PaneId};
use crate::spaces::SpaceInfo;

/// Longest tab label, in columns.
const MAX_LABEL: usize = 24;

/// What runs in a pane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Shell,
    /// The user's editor (`$VISUAL`, `$EDITOR`) on this file.
    Editor(PathBuf),
}

pub struct Slot {
    pub pane: Pane,
    pub kind: Kind,
}

impl Slot {
    /// An editor by its file; a shell by the title its program set, or its name.
    pub fn title(&self) -> String {
        let title = match &self.kind {
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
    Files,
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
    /// The file list, its right border included.
    pub files: Option<Rect>,
    pub panes: Layout,
    pub status: Rect,
}

impl SpaceLayout {
    /// The rows the file list's entries take (below its title row).
    pub fn file_rows(&self) -> Option<Rect> {
        self.files.map(|f| Rect {
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
    pub files_shown: bool,
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
            files_shown: false,
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
    pub fn right_text(&self) -> String {
        let trust = match (&self.info.root, self.info.trusted) {
            (None, _) => "no folder",
            (Some(_), true) => "trusted",
            (Some(_), false) => "not trusted",
        };
        format!("{} · {trust} ", self.place)
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
        let right = u16::try_from(self.right_text().width())
            .unwrap_or(u16::MAX)
            .min(area.width / 3);
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

        let (files, panes_area) = self.split_body(body);
        let panes = match self.tab() {
            Some(tab) if tab.zoomed => Tree::Pane(tab.focused).layout(panes_area),
            Some(tab) => tab.tree.layout(panes_area),
            None => Layout::default(),
        };
        SpaceLayout {
            header,
            labels,
            files,
            panes,
            status,
        }
    }

    /// The body (between the header and the status bar) divided between the
    /// file list, when shown, and the panes.
    fn split_body(&self, body: Rect) -> (Option<Rect>, Rect) {
        let files = (self.files_shown && self.files.is_some() && body.width >= 40).then(|| Rect {
            width: (body.width / 4).clamp(18, 36),
            ..body
        });
        let panes = match files {
            Some(files) => Rect {
                x: files.right(),
                width: body.width - files.width,
                ..body
            },
            None => body,
        };
        (files, panes)
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

    /// Whether a program other than a shell at its prompt is running, by pane.
    pub fn busy(&self) -> impl Iterator<Item = &Slot> {
        self.slots.iter().filter(|s| s.pane.is_busy())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_titles_are_clipped() {
        assert_eq!(clip("main.rs", 10), "main.rs");
        assert_eq!(clip("a-very-long-file-name.rs", 10), "a-very-lo…");
        assert_eq!(clip("日本語のファイル", 7), "日本語…");
    }
}
