//! The file list: a space's folder as a tree, read one folder at a time
//! through `x8ai-workspace` (so it never leaves the space), as the app's
//! explorer does. Only folders the user opens are read.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use x8ai_core::workspace::EntryKind;
use x8ai_workspace::Workspace;

/// Most rows listed, however many folders are open.
const MAX_ROWS: usize = 20_000;

/// Deepest folder listed: a symlink can lead back up inside the space.
const MAX_DEPTH: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    /// Workspace path, relative to the space's folder.
    pub path: String,
    pub folder: bool,
    pub open: bool,
    pub depth: usize,
}

/// What Enter (or a click) on a row did.
#[derive(Debug, PartialEq, Eq)]
pub enum Activated {
    Folder,
    File(PathBuf),
    Nothing,
}

pub struct FileList {
    workspace: Workspace,
    open: BTreeSet<String>,
    rows: Vec<Row>,
    selected: usize,
    /// The first row shown.
    offset: usize,
    error: Option<String>,
}

impl FileList {
    pub fn new(workspace: Workspace) -> Self {
        let mut list = Self {
            workspace,
            open: BTreeSet::new(),
            rows: Vec::new(),
            selected: 0,
            offset: 0,
            error: None,
        };
        list.reload();
        list
    }

    pub fn root(&self) -> &Path {
        self.workspace.root()
    }

    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Why the folder could not be read, if it could not.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Reads the open folders again, after a change on disk. The selection
    /// stays on the same entry while it exists.
    pub fn reload(&mut self) {
        let was = self.rows.get(self.selected).map(|r| r.path.clone());
        let mut rows = Vec::new();
        self.error = None;
        self.add_folder("", 0, &mut rows);
        self.rows = rows;
        self.selected = was
            .and_then(|path| self.rows.iter().position(|r| r.path == path))
            .unwrap_or(self.selected)
            .min(self.rows.len().saturating_sub(1));
        self.offset = self.offset.min(self.rows.len().saturating_sub(1));
    }

    fn add_folder(&mut self, path: &str, depth: usize, rows: &mut Vec<Row>) {
        match self.workspace.list_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    if rows.len() >= MAX_ROWS {
                        return;
                    }
                    let folder = entry.kind == EntryKind::Directory;
                    let open = folder && depth < MAX_DEPTH && self.open.contains(&entry.path);
                    rows.push(Row {
                        name: entry.name,
                        path: entry.path.clone(),
                        folder,
                        open,
                        depth,
                    });
                    if open {
                        self.add_folder(&entry.path, depth + 1, rows);
                    }
                }
            }
            // The space's folder itself: say why it is empty.
            Err(error) if path.is_empty() => self.error = Some(error.to_string()),
            // A folder that went away, or cannot be read: closed.
            Err(_) => {
                self.open.remove(path);
            }
        }
    }

    pub fn move_by(&mut self, delta: isize) {
        let last = self.rows.len().saturating_sub(1);
        self.selected = self.selected.saturating_add_signed(delta).min(last);
    }

    pub fn select(&mut self, row: usize) {
        if row < self.rows.len() {
            self.selected = row;
        }
    }

    pub fn first(&mut self) {
        self.selected = 0;
    }

    pub fn last(&mut self) {
        self.selected = self.rows.len().saturating_sub(1);
    }

    /// Enter: a folder opens or closes; a file is handed back to open.
    pub fn activate(&mut self) -> Activated {
        let Some(row) = self.rows.get(self.selected) else {
            return Activated::Nothing;
        };
        if row.folder {
            let path = row.path.clone();
            if !self.open.remove(&path) {
                self.open.insert(path);
            }
            self.reload();
            Activated::Folder
        } else {
            Activated::File(self.root().join(&row.path))
        }
    }

    /// →: opens a closed folder, or steps into an open one.
    pub fn expand(&mut self) {
        match self.rows.get(self.selected) {
            Some(row) if row.folder && !row.open => {
                self.open.insert(row.path.clone());
                self.reload();
            }
            Some(row) if row.open => self.move_by(1),
            _ => {}
        }
    }

    /// ←: closes an open folder, or steps out to the folder holding the entry.
    pub fn collapse(&mut self) {
        let Some(row) = self.rows.get(self.selected) else {
            return;
        };
        if row.open {
            self.open.remove(&row.path.clone());
            self.reload();
        } else if row.depth > 0 {
            let depth = row.depth;
            if let Some(parent) = self.rows[..self.selected]
                .iter()
                .rposition(|r| r.depth < depth)
            {
                self.selected = parent;
            }
        }
    }

    /// Scrolls so the selection shows in `height` rows.
    pub fn keep_in_view(&mut self, height: usize) {
        if height == 0 {
            return;
        }
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
        self.offset = self.offset.min(self.rows.len().saturating_sub(height));
    }

    /// Scrolls the view by `delta` rows (the mouse wheel), selection unchanged.
    pub fn scroll(&mut self, delta: isize, height: usize) {
        let most = self.rows.len().saturating_sub(height);
        self.offset = self.offset.saturating_add_signed(delta).min(most);
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn list() -> (tempfile::TempDir, FileList) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        fs::create_dir_all(root.join("src/bin")).unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();
        fs::write(root.join("src/bin/tool.rs"), "").unwrap();
        fs::write(root.join("README.md"), "").unwrap();
        fs::write(root.join(".env.example"), "").unwrap();
        let list = FileList::new(Workspace::open(root).unwrap());
        (temp, list)
    }

    fn shown(list: &FileList) -> Vec<String> {
        list.rows()
            .iter()
            .map(|r| {
                format!(
                    "{}{}{}",
                    "  ".repeat(r.depth),
                    r.name,
                    if r.folder { "/" } else { "" }
                )
            })
            .collect()
    }

    #[test]
    fn folders_come_first_and_open_one_level_at_a_time() {
        let (_temp, mut list) = list();
        assert_eq!(shown(&list), ["src/", ".env.example", "README.md"]);
        assert_eq!(list.activate(), Activated::Folder);
        assert_eq!(
            shown(&list),
            ["src/", "  bin/", "  main.rs", ".env.example", "README.md"]
        );
        list.move_by(2);
        let Activated::File(path) = list.activate() else {
            panic!("main.rs is a file");
        };
        assert!(path.ends_with("src/main.rs"));
        assert!(path.is_absolute());
    }

    #[test]
    fn arrows_open_close_and_step_out() {
        let (_temp, mut list) = list();
        list.expand();
        list.expand();
        assert_eq!(list.rows()[list.selected()].name, "bin");
        list.expand();
        list.expand();
        assert_eq!(list.rows()[list.selected()].name, "tool.rs");
        list.collapse();
        assert_eq!(list.rows()[list.selected()].name, "bin");
        list.collapse();
        assert_eq!(shown(&list)[1..3], ["  bin/", "  main.rs"]);
        list.collapse();
        assert_eq!(list.rows()[list.selected()].name, "src");
        list.collapse();
        assert_eq!(shown(&list), ["src/", ".env.example", "README.md"]);
    }

    #[test]
    fn changes_on_disk_show_after_a_reload_and_the_selection_stays() {
        let (temp, mut list) = list();
        list.last();
        assert_eq!(list.rows()[list.selected()].name, "README.md");
        fs::write(temp.path().join("Cargo.toml"), "").unwrap();
        list.reload();
        assert_eq!(
            shown(&list),
            ["src/", ".env.example", "Cargo.toml", "README.md"]
        );
        assert_eq!(list.rows()[list.selected()].name, "README.md");
        // An open folder that is removed closes.
        list.first();
        list.activate();
        fs::remove_dir_all(temp.path().join("src")).unwrap();
        list.reload();
        assert_eq!(shown(&list), [".env.example", "Cargo.toml", "README.md"]);
    }

    #[test]
    fn the_view_follows_the_selection() {
        let (_temp, mut list) = list();
        list.activate();
        list.last();
        list.keep_in_view(2);
        assert_eq!(list.offset(), 3);
        list.first();
        list.keep_in_view(2);
        assert_eq!(list.offset(), 0);
        list.scroll(10, 2);
        assert_eq!(list.offset(), 3);
    }
}
