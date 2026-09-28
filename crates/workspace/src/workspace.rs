use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

use cap_std::ambient_authority;
use cap_std::fs::{Dir, Metadata, MetadataExt, OpenOptions};
use x8ai_core::workspace::{DirEntry, EntryKind, FileContent, FileVersion, WorkspaceInfo};

use crate::error::ConflictReason;
use crate::{Error, path};

/// Largest file [`Workspace::read_text`] opens. The editor copes with larger
/// files, but moving them through IPC as one string stops being reasonable.
pub const MAX_TEXT_FILE_BYTES: u64 = 32 * 1024 * 1024;

/// A directory the user chose, and the only part of the filesystem reachable
/// through it.
///
/// Every operation runs through a `cap_std::fs::Dir` handle on the root, which
/// resolves paths beneath it and refuses anything that would leave it, including
/// through symlinks. Paths are also validated up front for clear errors
/// (`path.rs`).
pub struct Workspace {
    root: PathBuf,
    name: String,
    pub(crate) dir: Dir,
}

/// How [`Workspace::delete`] removes an entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    /// Move to the user's Trash, where it can be recovered. What the app uses.
    ToTrash,
    /// Delete immediately. Used by tests, which must not fill the user's Trash.
    Permanently,
}

impl Workspace {
    /// Opens `path` as a workspace. The root is canonicalized once, here.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let shown = path.display().to_string();
        let root = std::fs::canonicalize(path).map_err(|e| Error::io(&shown, e))?;
        if !root.is_dir() {
            return Err(Error::invalid(&shown, "is not a directory"));
        }
        let dir =
            Dir::open_ambient_dir(&root, ambient_authority()).map_err(|e| Error::io(&shown, e))?;
        let name = root.file_name().map_or_else(
            || root.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Ok(Self { root, name, dir })
    }

    /// Absolute path of the root. Used as the working directory for new terminal
    /// sessions; never for file operations.
    /// Reopens a workspace remembered by its root, as [`Workspace::open`]
    /// recorded it (canonical). Fails with [`Error::Moved`] if the path now
    /// resolves elsewhere, for example because the folder was replaced by a
    /// symlink, so a remembered path never leads to a folder the user did not
    /// choose.
    pub fn reopen(root: &Path) -> Result<Self, Error> {
        let workspace = Self::open(root)?;
        if workspace.root != root {
            return Err(Error::Moved {
                path: root.display().to_string(),
                now: workspace.root.display().to_string(),
            });
        }
        Ok(workspace)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Describes the workspace. Trust is recorded in a [`crate::TrustStore`], not
    /// here, so the caller supplies it.
    pub fn info(&self, trusted: bool) -> WorkspaceInfo {
        WorkspaceInfo {
            root: self.root.display().to_string(),
            name: self.name.clone(),
            trusted,
        }
    }

    /// The entries of one directory, directories first, then by name ignoring case.
    /// Only this level is read; the tree is never loaded as a whole.
    pub fn list_dir(&self, dir_path: &str) -> Result<Vec<DirEntry>, Error> {
        let rel = path::parse_dir(dir_path)?;
        let entries = self.dir.read_dir(rel).map_err(|e| Error::io(dir_path, e))?;
        let mut listed = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| Error::io(dir_path, e))?;
            // APFS names are always UTF-8. Elsewhere, a name we cannot represent
            // could not be opened again either, so it is left out.
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            let entry_path = path::join(dir_path, &name);
            let file_type = entry.file_type().map_err(|e| Error::io(&entry_path, e))?;
            let symlink = file_type.is_symlink();
            let kind = if symlink {
                // Follows the link, within the workspace only: a link that is
                // broken or leads outside cannot be opened.
                self.dir
                    .metadata(Path::new(&entry_path))
                    .map_or(EntryKind::Other, |m| kind_of(&m))
            } else if file_type.is_dir() {
                EntryKind::Directory
            } else if file_type.is_file() {
                EntryKind::File
            } else {
                EntryKind::Other
            };
            listed.push(DirEntry {
                name,
                path: entry_path,
                kind,
                symlink,
            });
        }
        listed.sort_by(|a, b| {
            (a.kind != EntryKind::Directory)
                .cmp(&(b.kind != EntryKind::Directory))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(listed)
    }

    /// Reads a UTF-8 text file with its version, for editing. Binary files are
    /// refused.
    pub fn read_text(&self, file_path: &str) -> Result<FileContent, Error> {
        self.read_text_limited(file_path, MAX_TEXT_FILE_BYTES)
    }

    pub(crate) fn read_text_limited(
        &self,
        file_path: &str,
        max: u64,
    ) -> Result<FileContent, Error> {
        let rel = path::parse_entry(file_path)?;
        // The version is taken before reading. If the file changes in between, the
        // version is stale, and the next save reports a conflict instead of
        // silently overwriting the change.
        let metadata = self
            .dir
            .metadata(rel)
            .map_err(|e| Error::io(file_path, e))?;
        if metadata.is_dir() {
            return Err(Error::invalid(file_path, "is a directory"));
        }
        if metadata.len() > max {
            return Err(Error::TooLarge {
                path: file_path.to_owned(),
                size: metadata.len(),
                max,
            });
        }
        let version = version_of(&metadata);
        let bytes = self.dir.read(rel).map_err(|e| Error::io(file_path, e))?;
        // A NUL byte marks a binary file, as for search, git and ripgrep, even when
        // the bytes happen to be valid UTF-8. Editing and saving one would corrupt it.
        if bytes.contains(&0) {
            return Err(Error::NotText(file_path.to_owned()));
        }
        let text = String::from_utf8(bytes).map_err(|_| Error::NotText(file_path.to_owned()))?;
        Ok(FileContent { text, version })
    }

    /// The current version of a file, or `None` if it does not exist.
    pub fn file_version(&self, file_path: &str) -> Result<Option<FileVersion>, Error> {
        let rel = path::parse_entry(file_path)?;
        match self.dir.metadata(rel) {
            Ok(metadata) => Ok(Some(version_of(&metadata))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::io(file_path, e)),
        }
    }

    /// Saves `text`. With `expected`, the save goes ahead only if the file on disk
    /// is still that version, so an external change is never silently overwritten.
    /// Without it (an explicit overwrite), the file is written regardless.
    ///
    /// An existing regular file is replaced atomically: a sibling temporary file is
    /// written, flushed to disk and renamed over it, so a failure never leaves a
    /// half-written file. Its permissions are kept. A symlink is written through.
    pub fn write_text(
        &self,
        file_path: &str,
        text: &str,
        expected: Option<&FileVersion>,
    ) -> Result<FileVersion, Error> {
        let rel = path::parse_entry(file_path)?;
        let current = match self.dir.metadata(rel) {
            Ok(metadata) => Some(metadata),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(Error::io(file_path, e)),
        };
        if let Some(expected) = expected {
            let reason = match &current {
                None => Some(ConflictReason::Deleted),
                Some(metadata) if version_of(metadata) != *expected => {
                    Some(ConflictReason::Modified)
                }
                Some(_) => None,
            };
            if let Some(reason) = reason {
                return Err(Error::Conflict {
                    path: file_path.to_owned(),
                    reason,
                });
            }
        }
        if current.as_ref().is_some_and(Metadata::is_dir) {
            return Err(Error::invalid(file_path, "is a directory"));
        }

        let is_symlink = self
            .dir
            .symlink_metadata(rel)
            .is_ok_and(|m| m.file_type().is_symlink());
        match current {
            Some(metadata) if !is_symlink => self.replace(file_path, rel, text, &metadata)?,
            _ => self
                .dir
                .write(rel, text)
                .map_err(|e| Error::io(file_path, e))?,
        }
        let saved = self
            .dir
            .metadata(rel)
            .map_err(|e| Error::io(file_path, e))?;
        Ok(version_of(&saved))
    }

    fn replace(
        &self,
        file_path: &str,
        rel: &Path,
        text: &str,
        original: &Metadata,
    ) -> Result<(), Error> {
        static SAVES: AtomicU64 = AtomicU64::new(0);
        let name = rel
            .file_name()
            .map_or_else(Default::default, |n| n.to_string_lossy());
        let temp = rel.with_file_name(format!(
            ".{name}.x8ai-save-{}-{}",
            std::process::id(),
            SAVES.fetch_add(1, Ordering::Relaxed)
        ));
        let written = (|| {
            let mut file = self.dir.create(&temp)?;
            file.write_all(text.as_bytes())?;
            file.sync_all()?;
            drop(file);
            self.dir.set_permissions(&temp, original.permissions())?;
            self.dir.rename(&temp, &self.dir, rel)
        })();
        if written.is_err() {
            let _ = self.dir.remove_file(&temp);
        }
        written.map_err(|e| Error::io(file_path, e))
    }

    /// Creates an empty file. Fails if anything already exists there.
    pub fn create_file(&self, file_path: &str) -> Result<(), Error> {
        let rel = path::parse_entry(file_path)?;
        path::check_name(leaf(file_path))?;
        self.dir
            .open_with(rel, OpenOptions::new().write(true).create_new(true))
            .map(drop)
            .map_err(|e| Error::io(file_path, e))
    }

    /// Creates a directory. Its parent must exist.
    pub fn create_dir(&self, dir_path: &str) -> Result<(), Error> {
        let rel = path::parse_entry(dir_path)?;
        path::check_name(leaf(dir_path))?;
        self.dir.create_dir(rel).map_err(|e| Error::io(dir_path, e))
    }

    /// Renames or moves an entry. Never replaces an existing entry, except that a
    /// name can change only in case (`readme.md` → `README.md`) on case-insensitive
    /// filesystems.
    pub fn rename(&self, from: &str, to: &str) -> Result<(), Error> {
        let source = path::parse_entry(from)?;
        let target = path::parse_entry(to)?;
        path::check_name(leaf(to))?;
        let source_metadata = self
            .dir
            .symlink_metadata(source)
            .map_err(|e| Error::io(from, e))?;
        match self.dir.symlink_metadata(target) {
            Ok(existing) if !same_entry(&existing, &source_metadata) => {
                return Err(Error::AlreadyExists(to.to_owned()));
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(Error::io(to, e)),
        }
        self.dir
            .rename(source, &self.dir, target)
            .map_err(|e| Error::io(from, e))
    }

    /// Deletes a file or a directory with its contents. A symlink is removed
    /// itself; its target is untouched.
    pub fn delete(&self, entry_path: &str, how: Removal) -> Result<(), Error> {
        let rel = path::parse_entry(entry_path)?;
        // Confirms the entry exists, and that its path resolves inside the
        // workspace, before anything is removed.
        let metadata = self
            .dir
            .symlink_metadata(rel)
            .map_err(|e| Error::io(entry_path, e))?;
        match how {
            Removal::Permanently if metadata.is_dir() => self.dir.remove_dir_all(rel),
            Removal::Permanently => self.dir.remove_file(rel),
            Removal::ToTrash => return trash(&self.root.join(rel), entry_path),
        }
        .map_err(|e| Error::io(entry_path, e))
    }
}

fn trash(absolute: &Path, entry_path: &str) -> Result<(), Error> {
    #[cfg(target_os = "macos")]
    let context = {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        // The default method drives Finder through `osascript`, which needs
        // Automation permission and runs a helper process. NSFileManager is a
        // direct system call.
        let mut context = trash::TrashContext::default();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context
    };
    #[cfg(not(target_os = "macos"))]
    let context = trash::TrashContext::default();

    context.delete(absolute).map_err(|e| Error::Io {
        path: entry_path.to_owned(),
        detail: format!("could not move to the Trash: {e}"),
    })
}

fn leaf(entry_path: &str) -> &str {
    entry_path.rsplit('/').next().unwrap_or(entry_path)
}

fn kind_of(metadata: &Metadata) -> EntryKind {
    if metadata.is_dir() {
        EntryKind::Directory
    } else if metadata.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    }
}

fn same_entry(a: &Metadata, b: &Metadata) -> bool {
    a.dev() == b.dev() && a.ino() == b.ino()
}

/// Modification time (to the nanosecond) and size.
fn version_of(metadata: &Metadata) -> FileVersion {
    let modified = metadata
        .modified()
        .ok()
        .and_then(|t| t.into_std().duration_since(UNIX_EPOCH).ok())
        .map_or_else(
            || "unknown".to_owned(),
            |d| format!("{}.{:09}", d.as_secs(), d.subsec_nanos()),
        );
    FileVersion(format!("{modified}:{}", metadata.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_files_over_the_size_limit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("big.txt"), vec![b'x'; 2048]).unwrap();
        let workspace = Workspace::open(dir.path()).unwrap();
        assert!(matches!(
            workspace.read_text_limited("big.txt", 1024),
            Err(Error::TooLarge {
                size: 2048,
                max: 1024,
                ..
            })
        ));
        assert!(workspace.read_text_limited("big.txt", 4096).is_ok());
    }
}
