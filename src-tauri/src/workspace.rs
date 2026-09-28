//! Workspace commands: the IPC face of `x8ai-workspace`.
//!
//! The webview cannot name a folder to open. `workspace_open` shows the native
//! picker, and whatever the user chooses becomes the only part of the filesystem
//! these commands can reach. Every other command takes workspace paths, which
//! `x8ai-workspace` validates and resolves beneath the root.

use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use tauri::ipc::Channel;
use tauri::{State, WebviewWindow};
use tauri_plugin_dialog::DialogExt;
use x8ai_core::error::{CommandError, ErrorCode};
use x8ai_core::workspace::{
    DirEntry, FileContent, FileList, FileVersion, WorkspaceEvent, WorkspaceInfo,
};
use x8ai_workspace::{Removal, Watcher, Workspace};

/// Quick open lists at most this many files.
const MAX_LISTED_FILES: usize = 50_000;

/// The open workspace, if any. Managed Tauri state.
#[derive(Default)]
pub struct Workspaces(Mutex<Option<Open>>);

struct Open {
    workspace: Arc<Workspace>,
    _watcher: Watcher,
}

impl Workspaces {
    pub fn current(&self) -> Option<Arc<Workspace>> {
        self.lock().as_ref().map(|open| open.workspace.clone())
    }

    /// The root of the open workspace, where new terminal sessions start.
    pub fn root(&self) -> Option<PathBuf> {
        self.current().map(|workspace| workspace.root().to_owned())
    }

    /// Forgets the workspace and stops watching it. Called when the page reloads.
    pub fn close(&self) {
        self.lock().take();
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Open>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Shows the native folder picker. The chosen folder replaces the open workspace,
/// and changes on disk are reported on `events`. `None` if the user cancels, in
/// which case the current workspace stays open.
#[tauri::command]
pub async fn workspace_open(
    window: WebviewWindow,
    events: Channel<WorkspaceEvent>,
    workspaces: State<'_, Workspaces>,
) -> Result<Option<WorkspaceInfo>, CommandError> {
    let picked = window
        .dialog()
        .file()
        .set_parent(&window)
        .set_title("Open Folder")
        .blocking_pick_folder();
    let Some(picked) = picked else {
        return Ok(None);
    };
    let path = picked
        .into_path()
        .map_err(|e| CommandError::new(ErrorCode::InvalidInput, format!("unusable folder: {e}")))?;

    let (workspace, watcher) = tauri::async_runtime::spawn_blocking(move || {
        let workspace = Workspace::open(&path)?;
        // Sending fails only when the page that opened the workspace is gone; the
        // workspace is closed when the page reloads (see `lib.rs`).
        let watcher = workspace.watch(move |event| drop(events.send(event)))?;
        Ok::<_, x8ai_workspace::Error>((workspace, watcher))
    })
    .await
    .map_err(internal)?
    .map_err(command_error)?;

    let info = workspace.info();
    *workspaces.lock() = Some(Open {
        workspace: Arc::new(workspace),
        _watcher: watcher,
    });
    Ok(Some(info))
}

#[tauri::command]
pub async fn workspace_list_dir(
    path: String,
    workspaces: State<'_, Workspaces>,
) -> Result<Vec<DirEntry>, CommandError> {
    run(&workspaces, move |w| w.list_dir(&path)).await
}

#[tauri::command]
pub async fn workspace_read_file(
    path: String,
    workspaces: State<'_, Workspaces>,
) -> Result<FileContent, CommandError> {
    run(&workspaces, move |w| w.read_text(&path)).await
}

#[tauri::command]
pub async fn workspace_file_version(
    path: String,
    workspaces: State<'_, Workspaces>,
) -> Result<Option<FileVersion>, CommandError> {
    run(&workspaces, move |w| w.file_version(&path)).await
}

/// Saves a file. With `expected`, fails with `conflict` if the file on disk is no
/// longer that version; with `None`, overwrites.
#[tauri::command]
pub async fn workspace_write_file(
    path: String,
    text: String,
    expected: Option<FileVersion>,
    workspaces: State<'_, Workspaces>,
) -> Result<FileVersion, CommandError> {
    run(&workspaces, move |w| {
        w.write_text(&path, &text, expected.as_ref())
    })
    .await
}

#[tauri::command]
pub async fn workspace_create_file(
    path: String,
    workspaces: State<'_, Workspaces>,
) -> Result<(), CommandError> {
    run(&workspaces, move |w| w.create_file(&path)).await
}

#[tauri::command]
pub async fn workspace_create_dir(
    path: String,
    workspaces: State<'_, Workspaces>,
) -> Result<(), CommandError> {
    run(&workspaces, move |w| w.create_dir(&path)).await
}

#[tauri::command]
pub async fn workspace_rename(
    from: String,
    to: String,
    workspaces: State<'_, Workspaces>,
) -> Result<(), CommandError> {
    run(&workspaces, move |w| w.rename(&from, &to)).await
}

/// Moves the entry to the Trash, so a mistaken delete can be undone in Finder.
#[tauri::command]
pub async fn workspace_delete(
    path: String,
    workspaces: State<'_, Workspaces>,
) -> Result<(), CommandError> {
    run(&workspaces, move |w| w.delete(&path, Removal::ToTrash)).await
}

#[tauri::command]
pub async fn workspace_list_files(
    workspaces: State<'_, Workspaces>,
) -> Result<FileList, CommandError> {
    run(&workspaces, |w| w.list_files(MAX_LISTED_FILES)).await
}

/// Runs a filesystem operation on a blocking thread, so large reads and writes
/// never hold up the IPC runtime.
async fn run<T: Send + 'static>(
    workspaces: &Workspaces,
    operation: impl FnOnce(&Workspace) -> Result<T, x8ai_workspace::Error> + Send + 'static,
) -> Result<T, CommandError> {
    let workspace = workspaces
        .current()
        .ok_or_else(|| CommandError::new(ErrorCode::NotFound, "no workspace is open"))?;
    tauri::async_runtime::spawn_blocking(move || operation(&workspace))
        .await
        .map_err(internal)?
        .map_err(command_error)
}

fn command_error(error: x8ai_workspace::Error) -> CommandError {
    use x8ai_workspace::Error;
    let code = match &error {
        Error::InvalidPath { .. } | Error::NotText(_) | Error::TooLarge { .. } => {
            ErrorCode::InvalidInput
        }
        Error::NotFound(_) => ErrorCode::NotFound,
        Error::AlreadyExists(_) => ErrorCode::AlreadyExists,
        Error::PermissionDenied { .. } => ErrorCode::PermissionDenied,
        Error::Conflict { .. } => ErrorCode::Conflict,
        Error::Io { .. } => ErrorCode::Internal,
    };
    CommandError::new(code, error.to_string())
}

fn internal(error: impl std::fmt::Display) -> CommandError {
    CommandError::new(ErrorCode::Internal, error.to_string())
}
