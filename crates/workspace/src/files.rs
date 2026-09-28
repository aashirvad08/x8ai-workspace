use std::path::Path;

use x8ai_core::workspace::FileList;

use crate::{Error, Workspace};

impl Workspace {
    /// Every file in the workspace, for quick open, up to `limit`. Walked on demand
    /// and never stored. Respects `.gitignore` (even outside a git repository),
    /// skips `.git`, includes other dotfiles, and does not follow symlinks.
    /// Directories that cannot be read are skipped.
    pub fn list_files(&self, limit: usize) -> Result<FileList, Error> {
        let walker = ignore::WalkBuilder::new(self.root())
            .hidden(false)
            .require_git(false)
            .follow_links(false)
            .filter_entry(|entry| entry.file_name() != ".git")
            .build();
        let mut paths = Vec::new();
        let mut truncated = false;
        for entry in walker.flatten() {
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                continue;
            }
            if paths.len() == limit {
                truncated = true;
                break;
            }
            if let Some(path) = relative(self.root(), entry.path()) {
                paths.push(path);
            }
        }
        paths.sort();
        Ok(FileList { paths, truncated })
    }
}

/// The workspace path of `path` under `root`, or `None` if it is not under the root
/// or is not UTF-8. The root itself is the empty path.
pub(crate) fn relative(root: &Path, path: &Path) -> Option<String> {
    let rest = path.strip_prefix(root).ok()?;
    let parts: Option<Vec<&str>> = rest.components().map(|c| c.as_os_str().to_str()).collect();
    Some(parts?.join("/"))
}
