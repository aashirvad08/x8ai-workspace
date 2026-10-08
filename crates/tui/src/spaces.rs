//! Spaces as `x8ai` opens them: through the desktop app's own stores, in its
//! data folder, so both share the recent spaces, their ids and trust.
//!
//! Each change reads the store from disk first and writes it straight back, so
//! the app running at the same time loses as little as possible: whichever
//! writes last wins, as with two windows of one app.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use x8ai_agents::{Authorized, Denied, LaunchPlan};
use x8ai_core::workspace::RecentWorkspace;
use x8ai_workspace::{
    ApprovalStore, Error, NEW_SPACE_NAME_RULE, NEW_SPACES_FOLDER, RecentWorkspaces, Space,
    SpaceStore, TrustStore, Workspace, new_space_name,
};

use crate::welcome::{MAX_NAME_LENGTH, tilde};

/// The desktop app's bundle identifier, which names its data folder.
const APP_IDENTIFIER: &str = "com.x8ai.workspace";

/// What `x8ai` keeps of its own, beside the app's stores.
const SETTINGS_FILE: &str = "terminal.json";

/// The app's stores of trusted folders and agent approvals.
const TRUST_FILE: &str = "trusted-workspaces.json";
const SPACES_FILE: &str = "spaces.json";
const APPROVALS_FILE: &str = "agent-approvals.json";

/// An open space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceInfo {
    /// `None` only when the data folder cannot be written.
    pub id: Option<String>,
    /// `None`: the workspace with no folder, in the home folder.
    pub root: Option<PathBuf>,
    pub name: String,
    pub trusted: bool,
}

pub struct Spaces {
    data_dir: PathBuf,
    home: PathBuf,
    warnings: Vec<String>,
}

impl Spaces {
    pub fn new(data_dir: PathBuf, home: PathBuf) -> Self {
        Self {
            data_dir,
            home,
            warnings: Vec::new(),
        }
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    /// The app's data folder.
    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    /// Problems reading or writing the stores since last asked. The space
    /// still opens; only remembering it failed.
    pub fn take_warnings(&mut self) -> Vec<String> {
        std::mem::take(&mut self.warnings)
    }

    /// The recent spaces, most recent first, with their ids.
    pub fn recent(&mut self) -> Vec<RecentWorkspace> {
        let recent = self.load(RecentWorkspaces::load, "recent-workspaces.json");
        let spaces = self.load(SpaceStore::load, SPACES_FILE);
        recent
            .list()
            .into_iter()
            .map(|r| RecentWorkspace {
                id: spaces.get(Some(Path::new(&r.root))).map(|s| s.id),
                ..r
            })
            .collect()
    }

    /// A folder the user typed, or gave to `x8ai`, opened as it is now.
    pub fn open(&mut self, path: &Path) -> Result<SpaceInfo, String> {
        let workspace = Workspace::open(path).map_err(|e| e.to_string())?;
        Ok(self.opened(Some(workspace.root())))
    }

    /// A recent space, only if its path still leads to the folder first
    /// chosen. One that is gone, or now leads elsewhere, leaves Recent.
    pub fn reopen(&mut self, root: &Path) -> Result<SpaceInfo, String> {
        match Workspace::reopen(root) {
            Ok(workspace) => Ok(self.opened(Some(workspace.root()))),
            Err(error @ (Error::NotFound(_) | Error::InvalidPath { .. } | Error::Moved { .. })) => {
                let reason = if matches!(error, Error::Moved { .. }) {
                    error.to_string()
                } else {
                    format!("{} no longer exists", tilde(root, &self.home))
                };
                let mut recent = self.load(RecentWorkspaces::load, "recent-workspaces.json");
                self.remember(recent.remove(root));
                Err(format!(
                    "{reason}, so it was removed from your recent spaces."
                ))
            }
            Err(error) => Err(error.to_string()),
        }
    }

    /// `/new <name>`: a new, empty folder `~/Workspaces/<name>`, opened. Never
    /// a folder that exists already.
    pub fn create(&mut self, name: &str) -> Result<SpaceInfo, String> {
        let name = new_space_name(name).ok_or(NEW_SPACE_NAME_RULE)?;
        let parent = self.home.join(NEW_SPACES_FOLDER);
        let path = parent.join(name);
        let shown = format!("~/{NEW_SPACES_FOLDER}/{name}");
        fs::create_dir_all(&parent)
            .and_then(|()| fs::create_dir(&path))
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::AlreadyExists {
                    format!("{shown} already exists: open it with /cd {shown}")
                } else {
                    format!("could not make {shown}: {e}")
                }
            })?;
        self.open(&path)
    }

    /// The workspace with no folder: terminals start in the home folder.
    pub fn home_space(&mut self) -> SpaceInfo {
        self.opened(None)
    }

    /// Records an opened space: first in Recent (a folder), and given an id
    /// if it has none.
    fn opened(&mut self, root: Option<&Path>) -> SpaceInfo {
        if let Some(root) = root {
            let mut recent = self.load(RecentWorkspaces::load, "recent-workspaces.json");
            self.remember(recent.record(root));
        }
        let mut spaces = self.load(SpaceStore::load, SPACES_FILE);
        let id = spaces.ensure(root).map(|s| s.id);
        let id = self.remember(id);
        let trusted = root.is_some_and(|root| self.is_trusted(root));
        let name = root.map_or_else(
            || "Home".to_owned(),
            |root| {
                root.file_name().map_or_else(
                    || root.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            },
        );
        SpaceInfo {
            id,
            root: root.map(Path::to_owned),
            name,
            trusted,
        }
    }

    /// The space of `root` (`None`: the workspace with no folder), with its
    /// add-ons, given an id now if it has none.
    pub fn space(&mut self, root: Option<&Path>) -> Result<Space, String> {
        self.load(SpaceStore::load, SPACES_FILE)
            .ensure(root)
            .map_err(|e| e.to_string())
    }

    /// Every space with an id.
    pub fn all_spaces(&mut self) -> Vec<Space> {
        self.load(SpaceStore::load, SPACES_FILE).list()
    }

    /// Adds add-ons to the space of `root`, after those it has.
    pub fn add_addons(&mut self, root: Option<&Path>, addons: &[&str]) -> Result<Space, String> {
        self.load(SpaceStore::load, SPACES_FILE)
            .add(root, addons)
            .map_err(|e| e.to_string())
    }

    /// Adds add-ons to the space with id `id`, as sharing does.
    pub fn add_addons_to(&mut self, id: &str, addons: &[&str]) -> Result<Space, String> {
        self.load(SpaceStore::load, SPACES_FILE)
            .add_to(id, addons)
            .map_err(|e| e.to_string())
    }

    pub fn remove_addon(&mut self, root: Option<&Path>, addon: &str) -> Result<Space, String> {
        self.load(SpaceStore::load, SPACES_FILE)
            .remove(root, addon)
            .map_err(|e| e.to_string())
    }

    /// The trusted folders, as the app's store holds them now.
    pub fn trust_store(&mut self) -> TrustStore {
        self.load(TrustStore::load, TRUST_FILE)
    }

    /// Whether the user trusted exactly this folder (ADR 0010).
    pub fn is_trusted(&mut self, root: &Path) -> bool {
        self.load(TrustStore::load, TRUST_FILE).is_trusted(root)
    }

    /// Trusts `root`, or stops trusting it. Without trust approvals mean
    /// nothing, so its agent and MCP approvals go with it, as in the app.
    pub fn set_trust(&mut self, root: &Path, trusted: bool) -> Result<(), String> {
        let mut trust = self.load(TrustStore::load, TRUST_FILE);
        trust.set(root, trusted).map_err(|e| e.to_string())?;
        if !trusted {
            let mut approvals = self.load(ApprovalStore::load, APPROVALS_FILE);
            approvals.revoke_all(root).map_err(|e| e.to_string())?;
            let (mut mcp, warning) =
                x8ai_mcp::Approvals::load(self.data_dir.join("mcp-approvals.json"));
            self.warnings.extend(warning);
            mcp.revoke_all(root).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Whether the user allowed exactly this launch: this agent, executable and
    /// arguments, in this folder (docs/agent-runtime.md).
    pub fn is_approved(&mut self, plan: &LaunchPlan) -> bool {
        self.load(ApprovalStore::load, APPROVALS_FILE)
            .is_approved(&plan.approval())
    }

    /// Whether the agent of `plan` is allowed in its folder at all, for any
    /// model configuration, as the app's Agents view shows it.
    pub fn is_approved_for_any_provider(&mut self, plan: &LaunchPlan) -> bool {
        self.load(ApprovalStore::load, APPROVALS_FILE)
            .is_approved_for_any_provider(&plan.approval())
    }

    /// Records the user's approval of `plan` in its folder.
    pub fn approve(&mut self, plan: &LaunchPlan) -> Result<(), String> {
        self.load(ApprovalStore::load, APPROVALS_FILE)
            .approve(&plan.approval())
            .map_err(|e| e.to_string())
    }

    /// Forgets `agent`'s approval in `root`.
    pub fn revoke(&mut self, root: &Path, agent: &str) -> Result<(), String> {
        self.load(ApprovalStore::load, APPROVALS_FILE)
            .revoke(root, agent)
            .map_err(|e| e.to_string())
    }

    /// Allows `plan` only if its folder is trusted and the user approved it
    /// there (`x8ai_agents::authorize`), as the stores say now.
    pub fn authorize<'a>(&mut self, plan: &'a LaunchPlan) -> Result<Authorized<'a>, Denied> {
        let trust = self.load(TrustStore::load, TRUST_FILE);
        let approvals = self.load(ApprovalStore::load, APPROVALS_FILE);
        x8ai_agents::authorize(plan, &trust, &approvals)
    }

    fn load<T>(&mut self, load: fn(PathBuf) -> (T, Option<String>), file: &str) -> T {
        let (store, warning) = load(self.data_dir.join(file));
        self.warnings.extend(warning);
        store
    }

    fn remember<T>(&mut self, result: Result<T, Error>) -> Option<T> {
        result
            .map_err(|e| self.warnings.push(format!("Could not save it: {e}")))
            .ok()
    }

    /// The name chosen with `/name`, if any.
    pub fn chosen_name(&self) -> Option<String> {
        let text = fs::read_to_string(self.data_dir.join(SETTINGS_FILE)).ok()?;
        let settings: Settings = serde_json::from_str(&text).ok()?;
        settings
            .name
            .map(|n| n.chars().take(MAX_NAME_LENGTH).collect::<String>())
            .filter(|n| !n.trim().is_empty())
    }

    /// Remembers the name the welcome greets; `None` goes back to the account's.
    pub fn choose_name(&mut self, name: Option<&str>) -> Result<(), String> {
        let settings = Settings {
            version: 1,
            name: name.map(str::to_owned),
        };
        let json = serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?;
        write_private(&self.data_dir.join(SETTINGS_FILE), &json)
            .map_err(|e| format!("Could not save your name: {e}"))
    }
}

#[derive(Serialize, Deserialize)]
struct Settings {
    version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
}

/// Replaces `file` whole, readable only by the user, in a folder only they
/// can open, as the app's stores are written.
fn write_private(file: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    let temp = file.with_extension(format!("json.tmp-{}", std::process::id()));
    let written = (|| {
        let mut out = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)?;
        out.write_all(bytes)?;
        out.sync_all()?;
        fs::rename(&temp, file)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

/// The desktop app's data folder (Tauri's `app_data_dir`), or
/// `X8AI_DATA_DIR` when set, which tests and trials use to stay apart.
pub fn data_dir(home: &Path) -> PathBuf {
    if let Some(dir) = std::env::var_os("X8AI_DATA_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if cfg!(target_os = "macos") {
        return home
            .join("Library/Application Support")
            .join(APP_IDENTIFIER);
    }
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local/share"))
        .join(APP_IDENTIFIER)
}

/// The account's full name, for the greeting, from the user database.
pub fn account_name() -> Option<String> {
    let account = nix::unistd::User::from_uid(nix::unistd::getuid()).ok()??;
    x8ai_core::app::account_name(account.gecos.to_str().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spaces() -> (tempfile::TempDir, Spaces) {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let spaces = Spaces::new(temp.path().join("data"), home);
        (temp, spaces)
    }

    #[test]
    fn an_opened_folder_is_recent_and_has_an_id() {
        let (_temp, mut spaces) = spaces();
        let folder = spaces.home().join("app");
        fs::create_dir(&folder).unwrap();
        let info = spaces.open(&folder).unwrap();
        assert_eq!(info.name, "app");
        assert!(!info.trusted);
        let id = info.id.clone().unwrap();
        assert!(x8ai_workspace::is_space_id(&id), "{id}");
        let recent = spaces.recent();
        assert_eq!(recent.len(), 1);
        assert_eq!(Path::new(&recent[0].root), info.root.as_deref().unwrap());
        assert_eq!(recent[0].id.as_deref(), Some(id.as_str()));
        // Opening it again keeps its id.
        assert_eq!(spaces.open(&folder).unwrap().id, Some(id));
        assert!(spaces.take_warnings().is_empty());
    }

    #[test]
    fn a_new_space_is_made_in_workspaces_once() {
        let (_temp, mut spaces) = spaces();
        let info = spaces.create(" demo ").unwrap();
        assert_eq!(info.name, "demo");
        assert!(spaces.home().join("Workspaces/demo").is_dir());
        let again = spaces.create("demo").unwrap_err();
        assert!(again.contains("already exists"), "{again}");
        assert_eq!(spaces.create("../etc").unwrap_err(), NEW_SPACE_NAME_RULE);
    }

    #[test]
    fn a_recent_space_that_is_gone_leaves_recent() {
        let (_temp, mut spaces) = spaces();
        let folder = spaces.home().join("gone");
        fs::create_dir(&folder).unwrap();
        let root = spaces.open(&folder).unwrap().root.unwrap();
        fs::remove_dir(&folder).unwrap();
        let error = spaces.reopen(&root).unwrap_err();
        assert!(error.contains("no longer exists"), "{error}");
        assert!(spaces.recent().is_empty());
    }

    #[test]
    fn the_home_space_has_an_id_and_no_folder() {
        let (_temp, mut spaces) = spaces();
        let home = spaces.home_space();
        assert_eq!(home.root, None);
        assert_eq!(home.name, "Home");
        assert_eq!(spaces.home_space().id, home.id);
        assert!(home.id.is_some());
        // The workspace with no folder is not a recent space.
        assert!(spaces.recent().is_empty());
    }

    #[test]
    fn the_chosen_name_is_kept_privately() {
        let (temp, mut spaces) = spaces();
        assert_eq!(spaces.chosen_name(), None);
        spaces.choose_name(Some("Ada")).unwrap();
        assert_eq!(spaces.chosen_name().as_deref(), Some("Ada"));
        let file = temp.path().join("data").join(SETTINGS_FILE);
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        spaces.choose_name(None).unwrap();
        assert_eq!(spaces.chosen_name(), None);
    }
}
