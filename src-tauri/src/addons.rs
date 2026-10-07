//! Add-on and space commands (ADR 0019): the IPC face of `x8ai-addons`.
//!
//! The webview names add-ons and spaces by id only. What an add-on installs and
//! runs is defined in `x8ai-addons`, never sent by the webview. Installing asks
//! first in a native dialog that shows the exact commands, which the webview
//! cannot answer; the install then runs once, in a terminal the user watches.
//! A space's terminals start with its add-ons (`terminal_env`); other spaces'
//! terminals, and the user's own shell files, are unchanged.

use std::collections::HashMap;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use x8ai_addons::{ADDONS, Addon, Mac, install_script, shown, terminal_setup, with_requirements};
use x8ai_agents::environment::var;
use x8ai_core::addon::{AddonAddResult, AddonList, AddonReach, AddonStatus, SpaceInfo};
use x8ai_core::error::{CommandError, ErrorCode};
use x8ai_core::terminal::{TerminalExit, TerminalInfo, TerminalSize};
use x8ai_pty::{Environment, Program, SessionEvents};
use x8ai_workspace::{Space, is_space_id};

use crate::agents::{Agents, home};
use crate::terminal::{ChannelEvents, Terminals, command_error, info};
use crate::workspace::Workspaces;

/// The longest a user's startup file can be and still be looked through for
/// add-ons it already turns on.
const MAX_STARTUP_BYTES: u64 = 256 * 1024;

/// Installs the user confirmed, until their terminal starts. Managed Tauri state.
#[derive(Default)]
pub struct Addons {
    pending: Mutex<HashMap<u32, Pending>>,
    next: AtomicU32,
}

/// A confirmed install: what it runs, and the space it adds to when it ends well.
struct Pending {
    addons: Vec<&'static str>,
    root: Option<PathBuf>,
    script: String,
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What is on this Mac, from the user's login environment (read once and kept,
/// or again with `refresh`).
fn mac(app: &AppHandle, refresh: bool) -> Mac {
    let environment = app.state::<Agents>().environment(refresh);
    Mac {
        home: home(),
        path: var(&environment.vars, "PATH").map(str::to_owned),
    }
}

/// Where the user's own zsh files are: their `ZDOTDIR`, or home.
fn user_zdotdir() -> PathBuf {
    std::env::var_os("ZDOTDIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(home)
}

fn zsh() -> bool {
    Path::new(&x8ai_pty::user_shell())
        .file_name()
        .is_some_and(|name| name == "zsh")
}

/// The user's own zsh startup files, joined, to see what they already turn on.
fn startup_text() -> String {
    let dir = user_zdotdir();
    [".zshenv", ".zprofile", ".zshrc", ".zlogin"]
        .iter()
        .filter_map(|name| {
            let file = dir.join(name);
            let size = std::fs::metadata(&file).ok()?.len();
            (size <= MAX_STARTUP_BYTES)
                .then(|| std::fs::read_to_string(file).ok())
                .flatten()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Whether an add-on of `space` is on in its new terminals.
fn active(addon: &Addon, mac: &Mac, trusted: bool, zsh: bool) -> bool {
    mac.installed(addon) && (!addon.needs_trust || trusted) && (addon.zshrc.is_none() || zsh)
}

/// The space with no folder is the user's home, which is theirs: it counts as
/// trusted for add-ons. A folder counts only once the user trusted it.
fn trusted(workspaces: &Workspaces, root: Option<&Path>) -> bool {
    root.is_none_or(|root| workspaces.is_trusted(root))
}

fn space_info(space: &Space) -> SpaceInfo {
    SpaceInfo {
        id: space.id.clone(),
        name: space.root.as_deref().map_or_else(
            || "Home".to_owned(),
            |root| {
                root.file_name().map_or_else(
                    || root.display().to_string(),
                    |n| n.to_string_lossy().into_owned(),
                )
            },
        ),
        root: space.root.as_ref().map(|r| r.display().to_string()),
        addons: space.addons.clone(),
    }
}

fn list(app: &AppHandle, refresh: bool) -> Result<AddonList, CommandError> {
    let workspaces = app.state::<Workspaces>();
    let root = workspaces.root();
    let space = workspaces.space(root.as_deref())?;
    let mac = mac(app, refresh);
    let trusted = trusted(&workspaces, root.as_deref());
    let zsh = zsh();
    let startup = if zsh { startup_text() } else { String::new() };
    let addons = ADDONS
        .iter()
        .map(|addon| {
            let installed = mac.installed(addon);
            let added = space.addons.iter().any(|a| a == addon.id);
            AddonStatus {
                id: addon.id.to_owned(),
                name: addon.name.to_owned(),
                description: addon.description.to_owned(),
                usage: addon.usage.map(str::to_owned),
                group: addon.group,
                reach: addon.reach,
                requires: addon.requires.iter().map(|r| (*r).to_owned()).collect(),
                installed,
                added,
                needs_trust: addon.needs_trust,
                active: added && active(addon, &mac, trusted, zsh),
                in_your_shell: zsh && x8ai_addons::in_your_shell(addon, &startup),
                install: if installed {
                    Vec::new()
                } else {
                    mac.install_commands(&with_requirements([addon.id]))
                        .map(|commands| commands.iter().map(|c| shown(c)).collect())
                        .unwrap_or_default()
                },
            }
        })
        .collect();
    Ok(AddonList {
        space: space_info(&space),
        homebrew: mac.brew().is_some(),
        shell: x8ai_pty::user_shell(),
        shell_supported: zsh,
        addons,
    })
}

async fn blocking<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce(&AppHandle) -> Result<T, CommandError> + Send + 'static,
) -> Result<T, CommandError> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || f(&app))
        .await
        .map_err(|e| CommandError::new(ErrorCode::Internal, e.to_string()))?
}

fn unknown(id: &str) -> CommandError {
    CommandError::new(ErrorCode::InvalidInput, format!("unknown add-on {id:?}"))
}

/// Every add-on, and its state in the open space. With `refresh`, the login
/// environment is read again first (after installing something outside the app).
#[tauri::command]
pub async fn addon_list(refresh: bool, app: AppHandle) -> Result<AddonList, CommandError> {
    blocking(&app, move |app| list(app, refresh)).await
}

/// Adds an add-on, with what it requires, to the open space. If something it
/// needs is not installed, a native dialog shows the commands that install it;
/// once the user confirms, the result carries a token for `addon_install`.
#[tauri::command]
pub async fn addon_add(
    id: String,
    window: WebviewWindow,
    app: AppHandle,
) -> Result<AddonAddResult, CommandError> {
    let addon = x8ai_addons::find(&id).ok_or_else(|| unknown(&id))?;
    let (space, commands) = blocking(&app, move |app| {
        let workspaces = app.state::<Workspaces>();
        let space = workspaces.space(workspaces.root().as_deref())?;
        let set = with_requirements([addon.id]);
        let commands = mac(app, false)
            .install_commands(&set)
            .map_err(|missing| CommandError::new(ErrorCode::NotFound, missing.to_string()))?;
        if commands.is_empty() {
            let ids: Vec<&str> = set.iter().map(|a| a.id).collect();
            workspaces.add_addons(space.root.as_deref(), &ids)?;
        }
        Ok((space, commands))
    })
    .await?;
    if commands.is_empty() {
        return Ok(AddonAddResult::Added {
            list: blocking(&app, |app| list(app, false)).await?,
        });
    }

    let space_name = space_info(&space).name;
    let lines: Vec<String> = commands
        .iter()
        .map(|c| format!("    {}", shown(c)))
        .collect();
    let reach = match addon.reach {
        AddonReach::Space => format!("It then turns on in {space_name}'s new terminals only."),
        AddonReach::Mac => format!(
            "It is a program for your whole Mac; it is added to {space_name}'s set of add-ons."
        ),
    };
    let confirmed = window
        .dialog()
        .message(format!(
            "It is installed first, in a terminal tab where you can watch it:\n\n{}\n\n\
             Homebrew downloads it from its own sources. {reach}",
            lines.join("\n")
        ))
        .title(format!("Add {} to {space_name}?", addon.name))
        .kind(MessageDialogKind::Info)
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Install".into(),
            "Cancel".into(),
        ))
        .parent(&window)
        .blocking_show();
    if !confirmed {
        return Ok(AddonAddResult::Cancelled);
    }
    let addons = app.state::<Addons>();
    let token = addons.next.fetch_add(1, Ordering::Relaxed).wrapping_add(1);
    lock(&addons.pending).insert(
        token,
        Pending {
            addons: with_requirements([addon.id]).iter().map(|a| a.id).collect(),
            root: space.root,
            script: install_script(&commands),
        },
    );
    Ok(AddonAddResult::Install {
        token,
        name: addon.name.to_owned(),
    })
}

/// Runs a confirmed install, once, in a new terminal session in the home folder.
/// When it ends well and everything is installed, the add-ons are added to the
/// space they were confirmed for. Output and exit arrive on `events` as for
/// `terminal_create`.
#[tauri::command]
pub async fn addon_install(
    token: u32,
    size: TerminalSize,
    events: Channel,
    app: AppHandle,
) -> Result<TerminalInfo, CommandError> {
    let pending = lock(&app.state::<Addons>().pending)
        .remove(&token)
        .ok_or_else(|| {
            CommandError::new(
                ErrorCode::NotFound,
                "this install already ran; add the add-on again to run it again",
            )
        })?;
    blocking(&app, move |app| {
        let environment = app.state::<Agents>().environment(false);
        let program = Program::Exec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), pending.script.clone().into()],
            cwd: Some(home()),
            env: Environment::Exactly(environment.vars.clone()),
        };
        let events = Arc::new(InstallEvents {
            channel: ChannelEvents(events),
            app: app.clone(),
            pending,
        });
        let session = app
            .state::<Terminals>()
            .sessions()
            .spawn(&program, size, events)
            .map_err(command_error)?;
        Ok(info(&session))
    })
    .await
}

/// An install's terminal events; when it ends well, the add-ons are added.
struct InstallEvents {
    channel: ChannelEvents,
    app: AppHandle,
    pending: Pending,
}

impl SessionEvents for InstallEvents {
    fn output(&self, bytes: Vec<u8>) {
        self.channel.output(bytes);
    }

    fn error(&self, message: String) {
        self.channel.error(message);
    }

    fn exited(&self, exit: TerminalExit) {
        if exit.code == 0 && exit.signal.is_none() {
            // Read again: what was installed may have changed the login PATH.
            let mac = mac(&self.app, true);
            let addons: Vec<&'static Addon> = self
                .pending
                .addons
                .iter()
                .filter_map(|id| x8ai_addons::find(id))
                .collect();
            if addons.iter().all(|a| mac.installed(a)) {
                let ids: Vec<&str> = addons.iter().map(|a| a.id).collect();
                let workspaces = self.app.state::<Workspaces>();
                if let Err(error) = workspaces.add_addons(self.pending.root.as_deref(), &ids) {
                    workspaces.warn(format!(
                        "Could not add {}: {}",
                        ids.join(", "),
                        error.message
                    ));
                }
            }
        }
        // Last, so the webview hears of the end once the add-on is added.
        self.channel.exited(exit);
    }
}

/// Takes an add-on out of the open space. Nothing is uninstalled: other spaces
/// may use it.
#[tauri::command]
pub async fn addon_remove(id: String, app: AppHandle) -> Result<AddonList, CommandError> {
    x8ai_addons::find(&id).ok_or_else(|| unknown(&id))?;
    blocking(&app, move |app| {
        let workspaces = app.state::<Workspaces>();
        workspaces.remove_addon(workspaces.root().as_deref(), &id)?;
        list(app, false)
    })
    .await
}

/// Every space with an id: the one with no folder first.
#[tauri::command]
pub fn space_list(workspaces: State<'_, Workspaces>) -> Vec<SpaceInfo> {
    workspaces.spaces().iter().map(space_info).collect()
}

/// Gives another space add-ons of the open one (`/share`): they are added to
/// what it has. Only add-ons the open space has can be shared.
#[tauri::command]
pub fn space_share(
    to: String,
    addons: Vec<String>,
    workspaces: State<'_, Workspaces>,
) -> Result<SpaceInfo, CommandError> {
    let from = workspaces.space(workspaces.root().as_deref())?;
    if !is_space_id(&to) || workspaces.spaces().iter().all(|s| s.id != to) {
        return Err(CommandError::new(
            ErrorCode::NotFound,
            format!("no space has the id {to:?}"),
        ));
    }
    if to == from.id {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            "that is this space; choose another one to share with",
        ));
    }
    if addons.is_empty() {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            "choose at least one add-on to share",
        ));
    }
    if let Some(stranger) = addons.iter().find(|a| !from.addons.contains(a)) {
        return Err(CommandError::new(
            ErrorCode::InvalidInput,
            format!("{stranger:?} is not an add-on of this space"),
        ));
    }
    let ids: Vec<&str> = addons.iter().map(String::as_str).collect();
    Ok(space_info(&workspaces.add_addons_to(&to, &ids)?))
}

/// The variables a new terminal of the open space starts with: its id, and its
/// active add-ons, whose zsh files are written to the space's own folder in the
/// app's data directory. A problem leaves the terminal as it would be without
/// add-ons, and is reported (`app_take_warnings`).
pub(crate) fn terminal_env(app: &AppHandle) -> Vec<(String, String)> {
    let workspaces = app.state::<Workspaces>();
    let root = workspaces.root();
    let space = match workspaces.space(root.as_deref()) {
        Ok(space) => space,
        Err(error) => {
            workspaces.warn(format!("This terminal has no add-ons: {}", error.message));
            return Vec::new();
        }
    };
    let mut env = vec![("X8AI_SPACE".to_owned(), space.id.clone())];
    if space.addons.is_empty() {
        return env;
    }
    let mac = mac(app, false);
    let trusted = trusted(&workspaces, root.as_deref());
    let zsh = zsh();
    let addons: Vec<&Addon> = ADDONS
        .iter()
        .filter(|a| space.addons.iter().any(|id| id == a.id) && active(a, &mac, trusted, zsh))
        .collect();
    let Some(data_dir) = workspaces.data_dir() else {
        return env;
    };
    let dir = data_dir.join("spaces").join(&space.id).join("zsh");
    let setup = terminal_setup(
        &format!("{} ({})", space_info(&space).name, space.id),
        &addons,
        &dir,
        &user_zdotdir(),
        &mac,
    );
    if !setup.files.is_empty()
        && let Err(error) = write_files(&data_dir.join("spaces"), &dir, &setup.files)
    {
        workspaces.warn(format!(
            "This terminal's shell add-ons are off: {} could not be written ({error})",
            dir.display()
        ));
        env.extend(
            setup
                .env
                .into_iter()
                .filter(|(name, _)| name != "ZDOTDIR" && name != "X8AI_USER_ZDOTDIR"),
        );
        return env;
    }
    env.extend(setup.env);
    env
}

/// Writes the space's zsh files, readable only by the user, each replaced whole.
fn write_files(spaces: &Path, dir: &Path, files: &[(&'static str, String)]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(spaces, std::fs::Permissions::from_mode(0o700))?;
    for (name, text) in files {
        let temp = dir.join(format!("{name}.tmp-{}", std::process::id()));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)
            .and_then(|mut out| std::io::Write::write_all(&mut out, text.as_bytes()))
            .and_then(|()| std::fs::rename(&temp, dir.join(name)));
        if written.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        written?;
    }
    Ok(())
}
