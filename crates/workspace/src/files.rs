use std::path::Path;

use ignore::WalkBuilder;
use x8ai_core::workspace::FileList;

use crate::{Error, Workspace};

/// Directories that hold dependencies, caches or build output. Quick open and
/// search skip them even when no `.gitignore` says so, because they are large
/// and almost never what the user is looking for.
pub(crate) const SKIPPED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "bower_components",
    "target",
    "dist",
    "build",
    "out",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".parcel-cache",
    ".cache",
    "coverage",
    ".venv",
    "venv",
    "__pycache__",
    ".pytest_cache",
    ".mypy_cache",
    ".tox",
    ".gradle",
    ".idea",
];

/// Walks the workspace for quick open and search: respects `.gitignore` (even
/// outside a git repository), skips [`SKIPPED_DIRS`], includes other dotfiles,
/// and never follows symlinks, so it cannot leave the root.
pub(crate) fn walker(root: &Path, max_file_bytes: Option<u64>) -> WalkBuilder {
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(false)
        .require_git(false)
        .follow_links(false)
        .max_filesize(max_file_bytes)
        .filter_entry(|entry| {
            !(entry.file_type().is_some_and(|t| t.is_dir())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| SKIPPED_DIRS.contains(&name)))
        });
    builder
}

impl Workspace {
    /// Every file in the workspace, for quick open, up to `limit`. Walked on demand
    /// and never stored. Directories that cannot be read are skipped.
    pub fn list_files(&self, limit: usize) -> Result<FileList, Error> {
        let mut paths = Vec::new();
        let mut truncated = false;
        for entry in walker(self.root(), None).build().flatten() {
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
