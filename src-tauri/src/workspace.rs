//! Workspace commands: the IPC face of `x8ai-workspace`.
//!
//! The webview cannot name a folder to open. A workspace is either chosen in the
//! native folder picker, or reopened from the recent list, which holds only
//! folders previously chosen that way. Opening one stops the agents that were
//! running in the previous one. Trust is granted only through a native
//! confirmation dialog, so the webview cannot grant it itself. Every other command
//! takes workspace paths, which `x8ai-workspace` validates and resolves beneath
//! the root.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use tauri::ipc::Channel;
use tauri::{State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use x8ai_agents::{Authorized, Denied, LaunchPlan};
use x8ai_core::error::{CommandError, ErrorCode};
use x8ai_core::workspace::{
    DirEntry, FileContent, FileList, FileVersion, RecentWorkspace, SearchEvent, SearchQuery,
    WorkspaceEvent, WorkspaceInfo,
};
use x8ai_workspace::{
    ApprovalStore, RecentWorkspaces, Removal, SearchLimits, TrustStore, Watcher, Workspace,
};

use crate::agents::Agents;
use crate::terminal::Terminals;

/// Quick open lists at most this many files.
const MAX_LISTED_FILES: usize = 50_000;
/// Longest search text accepted.
const MAX_QUERY_CHARS: usize = 1_000;

/// The open workspace, and what is remembered about workspaces between
/// launches. Managed Tauri state.
#[derive(Default)]
pub struct Workspaces {
    current: Mutex<Option<Open>>,
    stores: Mutex<Option<Stores>>,
    /// Set to cancel the running search.
    search: Mutex<Option<Arc<AtomicBool>>>,
    warnings: Mutex<Vec<String>>,
}

struct Open {
    workspace: Arc<Workspace>,
    _watcher: Watcher,
}

struct Stores {
    recent: RecentWorkspaces,
    trust: TrustStore,
    approvals: ApprovalStore,
}

impl Workspaces {
    /// Loads the recent list, trust and agent approvals from the app's data
    /// directory. Problems reading them are kept for the frontend to show
    /// (`app_take_warnings`).
    pub fn load_stores(&self, data_dir: &Path) {
        let (recent, recent_warning) =
            RecentWorkspaces::load(data_dir.join("recent-workspaces.json"));
        let (trust, trust_warning) = TrustStore::load(data_dir.join("trusted-workspaces.json"));
        let (approvals, approvals_warning) =
            ApprovalStore::load(data_dir.join("agent-approvals.json"));
        lock(&self.warnings).extend(
            recent_warning
                .into_iter()
                .chain(trust_warning)
                .chain(approvals_warning),
        );
        *lock(&self.stores) = Some(Stores {
            recent,
            trust,
            approvals,
        });
    }

    /// Keeps a problem for the frontend to show (`app_take_warnings`).
    pub fn warn(&self, message: String) {
        lock(&self.warnings).push(message);
    }

    pub fn take_warnings(&self) -> Vec<String> {
        std::mem::take(&mut *lock(&self.warnings))
    }

    pub fn current(&self) -> Option<Arc<Workspace>> {
        lock(&self.current)
            .as_ref()
            .map(|open| open.workspace.clone())
    }

    /// The root of the open workspace, where new terminal sessions start.
    pub fn root(&self) -> Option<PathBuf> {
        self.current().map(|workspace| workspace.root().to_owned())
    }

    /// Forgets the workspace, stops watching it and cancels its search. Called
    /// when the page reloads.
    pub fn close(&self) {
        self.cancel_search();
        lock(&self.current).take();
    }

    /// Whether the user explicitly trusted exactly this folder (ADR 0010).
    pub(crate) fn is_trusted(&self, root: &Path) -> bool {
        lock(&self.stores)
            .as_ref()
            .is_some_and(|s| s.trust.is_trusted(root))
    }

    /// Whether the user approved exactly this agent launch in its workspace.
    pub(crate) fn is_approved(&self, plan: &LaunchPlan) -> bool {
        lock(&self.stores)
            .as_ref()
            .is_some_and(|s| s.approvals.is_approved(&plan.approval()))
    }

    /// Allows the launch only if its workspace is trusted and it is approved there
    /// (`x8ai_agents::authorize`).
    pub(crate) fn authorize<'a>(&self, plan: &'a LaunchPlan) -> Result<Authorized<'a>, Denied> {
        let stores = lock(&self.stores);
        let Some(stores) = stores.as_ref() else {
            return Err(Denied::Untrusted(plan.workspace.clone()));
        };
        x8ai_agents::authorize(plan, &stores.trust, &stores.approvals)
    }

    /// Records the user's approval of this launch in its workspace.
    pub(crate) fn approve(&self, plan: &LaunchPlan) -> Result<(), CommandError> {
        self.with_stores(|s| s.approvals.approve(&plan.approval()))
    }

    pub(crate) fn revoke(&self, root: &Path, agent: &str) -> Result<(), CommandError> {
        self.with_stores(|s| s.approvals.revoke(root, agent))
    }

    fn with_stores<T>(
        &self,
        f: impl FnOnce(&mut Stores) -> Result<T, x8ai_workspace::Error>,
    ) -> Result<T, CommandError> {
        let mut stores = lock(&self.stores);
        let stores = stores.as_mut().ok_or_else(|| {
            CommandError::new(ErrorCode::Internal, "workspace settings are not loaded")
        })?;
        f(stores).map_err(command_error)
    }

    /// Makes `workspace` the open one: watches it, moves it to the front of the
    /// recent list, and reports it with its trust.
    fn install(
        &self,
        workspace: Workspace,
        events: Channel<WorkspaceEvent>,
    ) -> Result<WorkspaceInfo, CommandError> {
        // Sending fails only when the page that opened the workspace is gone; the
        // workspace is closed when the page reloads (see `lib.rs`).
        let watcher = workspace
            .watch(move |event| drop(events.send(event)))
            .map_err(command_error)?;
        let root = workspace.root().to_owned();
        let recorded = self.with_stores(|s| s.recent.record(&root));
        let info = workspace.info(self.is_trusted(&root));
        self.cancel_search();
        *lock(&self.current) = Some(Open {
            workspace: Arc::new(workspace),
            _watcher: watcher,
        });
        // The workspace is open either way; failing to remember it is only reported.
        if let Err(error) = recorded {
            self.warn(format!(
                "Could not add {} to Recent: {}",
                root.display(),
                error.message
            ));
        }
        Ok(info)
    }

    fn begin_search(&self) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        if let Some(previous) = lock(&self.search).replace(cancel.clone()) {
            previous.store(true, Ordering::Relaxed);
        }
        cancel
    }

    fn cancel_search(&self) {
        if let Some(running) = lock(&self.search).take() {
            running.store(true, Ordering::Relaxed);
        }
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
    agents: State<'_, Agents>,
    terminals: State<'_, Terminals>,
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
    let workspace = blocking(move || Workspace::open(&path)).await?;
    let info = workspaces.install(workspace, events)?;
    agents.stop_outside(&terminals, Path::new(&info.root));
    Ok(Some(info))
}

/// Reopens a folder from the recent list. Only folders in that list, which the
/// user chose earlier in the native picker, can be opened this way. A folder that
/// no longer exists is removed from the list.
#[tauri::command]
pub async fn workspace_open_recent(
    root: String,
    events: Channel<WorkspaceEvent>,
    workspaces: State<'_, Workspaces>,
    agents: State<'_, Agents>,
    terminals: State<'_, Terminals>,
) -> Result<WorkspaceInfo, CommandError> {
    let path = PathBuf::from(&root);
    if !workspaces.with_stores(|s| Ok(s.recent.contains(&path)))? {
        return Err(CommandError::new(
            ErrorCode::PermissionDenied,
            "only folders opened before can be reopened; use Open Folder",
        ));
    }
    let opening = path.clone();
    let reopened = tauri::async_runtime::spawn_blocking(move || Workspace::reopen(&opening))
        .await
        .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?;
    match reopened {
        Ok(workspace) => {
            let info = workspaces.install(workspace, events)?;
            agents.stop_outside(&terminals, Path::new(&info.root));
            Ok(info)
        }
        // Gone, no longer a folder, or now a different folder: not worth keeping.
        Err(
            error @ (x8ai_workspace::Error::NotFound(_)
            | x8ai_workspace::Error::InvalidPath { .. }
            | x8ai_workspace::Error::Moved { .. }),
        ) => {
            workspaces.with_stores(|s| s.recent.remove(&path))?;
            let reason = if matches!(error, x8ai_workspace::Error::Moved { .. }) {
                error.to_string()
            } else {
                format!("{root} no longer exists")
            };
            Err(CommandError::new(
                command_error(error).code,
                format!(
                    "{reason}, so it was removed from Recent. Use Open Folder to choose it again."
                ),
            ))
        }
        // Anything else (for example macOS privacy settings denying access) may
        // pass; the folder stays in the list.
        Err(error) => Err(command_error(error)),
    }
}

#[tauri::command]
pub fn workspace_recent(
    workspaces: State<'_, Workspaces>,
) -> Result<Vec<RecentWorkspace>, CommandError> {
    workspaces.with_stores(|s| Ok(s.recent.list()))
}

/// Removes a folder from the recent list. Its trust is not changed.
#[tauri::command]
pub fn workspace_forget_recent(
    root: String,
    workspaces: State<'_, Workspaces>,
) -> Result<(), CommandError> {
    workspaces.with_stores(|s| s.recent.remove(Path::new(&root)))
}

/// Trusts or stops trusting the open workspace. Trusting asks for confirmation in
/// a native dialog, which the webview cannot answer on the user's behalf; removing
/// trust never needs confirmation, and also removes the folder's agent approvals
/// and stops its running agents. Returns the workspace with its trust as it now
/// is (unchanged if the user declined).
#[tauri::command]
pub async fn workspace_set_trust(
    trusted: bool,
    window: WebviewWindow,
    workspaces: State<'_, Workspaces>,
    agents: State<'_, Agents>,
    terminals: State<'_, Terminals>,
) -> Result<WorkspaceInfo, CommandError> {
    let workspace = current(&workspaces)?;
    let root = workspace.root().to_owned();
    if trusted && !workspaces.is_trusted(&root) {
        let confirmed = window
            .dialog()
            .message(format!(
                "{}\n\nTrust a folder only if you trust the code in it. Future versions will \
                 let AI agents and tools run commands only in trusted folders. Untrusted \
                 folders never run anything automatically. You can remove trust at any time.",
                root.display()
            ))
            .title(format!("Trust “{}”?", workspace.info(false).name))
            .kind(MessageDialogKind::Warning)
            .buttons(MessageDialogButtons::OkCancelCustom(
                "Trust".into(),
                "Cancel".into(),
            ))
            .parent(&window)
            .blocking_show();
        if !confirmed {
            return Ok(workspace.info(false));
        }
    }
    workspaces.with_stores(|s| {
        s.trust.set(&root, trusted)?;
        // Without trust, approvals mean nothing; they are not kept for later.
        if trusted {
            Ok(())
        } else {
            s.approvals.revoke_all(&root)
        }
    })?;
    if !trusted {
        // Nothing keeps running in a folder the user no longer trusts.
        agents.stop_in(&terminals, &root);
    }
    Ok(workspace.info(workspaces.is_trusted(&root)))
}

/// Searches the workspace, streaming each file's matches and then a summary on
/// `events`. Starting a search cancels the previous one.
#[tauri::command]
pub async fn workspace_search(
    query: SearchQuery,
    events: Channel<SearchEvent>,
    workspaces: State<'_, Workspaces>,
) -> Result<(), CommandError> {
    if query.text.chars().count() > MAX_QUERY_CHARS {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            format!("search text is longer than {MAX_QUERY_CHARS} characters"),
        ));
    }
    let workspace = current(&workspaces)?;
    let cancel = workspaces.begin_search();
    blocking(move || {
        let summary =
            workspace.search(&query, SearchLimits::default(), &cancel, |path, matches| {
                // A closed page stops listening; the search is cancelled on reload.
                let _ = events.send(SearchEvent::File { path, matches });
            })?;
        let _ = events.send(SearchEvent::Done(summary));
        Ok(())
    })
    .await
}

#[tauri::command]
pub fn workspace_search_cancel(workspaces: State<'_, Workspaces>) {
    workspaces.cancel_search();
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

fn current(workspaces: &Workspaces) -> Result<Arc<Workspace>, CommandError> {
    workspaces
        .current()
        .ok_or_else(|| CommandError::new(ErrorCode::NotFound, "no workspace is open"))
}

/// Runs a filesystem operation on the open workspace, on a blocking thread, so
/// large reads and writes never hold up the IPC runtime.
async fn run<T: Send + 'static>(
    workspaces: &Workspaces,
    operation: impl FnOnce(&Workspace) -> Result<T, x8ai_workspace::Error> + Send + 'static,
) -> Result<T, CommandError> {
    let workspace = current(workspaces)?;
    blocking(move || operation(&workspace)).await
}

async fn blocking<T: Send + 'static>(
    operation: impl FnOnce() -> Result<T, x8ai_workspace::Error> + Send + 'static,
) -> Result<T, CommandError> {
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
        .map_err(command_error)
}

pub(crate) fn command_error(error: x8ai_workspace::Error) -> CommandError {
    use x8ai_workspace::Error;
    let code = match &error {
        Error::InvalidPath { .. }
        | Error::NotText(_)
        | Error::TooLarge { .. }
        | Error::InvalidQuery(_) => ErrorCode::InvalidInput,
        Error::NotFound(_) => ErrorCode::NotFound,
        Error::AlreadyExists(_) => ErrorCode::AlreadyExists,
        Error::PermissionDenied { .. } | Error::Moved { .. } => ErrorCode::PermissionDenied,
        Error::Conflict { .. } => ErrorCode::Conflict,
        Error::Io { .. } => ErrorCode::Internal,
    };
    CommandError::new(code, error.to_string())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}
