//! A space's terminals with its add-ons, the same in the app and in `x8ai`:
//! which added add-ons are on, the variables a terminal starts with, and the
//! zsh files written for them in the space's own folder (ADR 0019).

use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::mac::Mac;
use crate::registry::{ADDONS, Addon};
use crate::shell::terminal_setup;

/// Whether `addon`, added to a space, is on in its terminals: installed, the
/// folder trusted if it needs that, and the shell zsh if it changes the shell.
pub fn active(addon: &Addon, mac: &Mac, trusted: bool, zsh: bool) -> bool {
    mac.installed(addon) && (!addon.needs_trust || trusted) && (addon.zshrc.is_none() || zsh)
}

/// Whether `shell` (a path) is zsh, the shell add-ons turn on in.
pub fn is_zsh(shell: &str) -> bool {
    Path::new(shell)
        .file_name()
        .is_some_and(|name| name == "zsh")
}

/// Where the user's own zsh files are: their `ZDOTDIR`, or `home`.
pub fn user_zdotdir(home: &Path) -> PathBuf {
    std::env::var_os("ZDOTDIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.to_owned())
}

/// The longest a user's startup file can be and still be looked through for
/// add-ons it already turns on.
const MAX_STARTUP_BYTES: u64 = 256 * 1024;

/// The user's own zsh startup files, joined, to see what they already turn on
/// in every terminal ([`crate::in_your_shell`]).
pub fn startup_text(home: &Path) -> String {
    let dir = user_zdotdir(home);
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

/// A space, as its terminals need it.
pub struct SpaceTerminal<'a> {
    pub id: &'a str,
    /// Named in the zsh files written for it: `app (ws-k3f9qa)`.
    pub title: &'a str,
    /// The add-ons added to it, by id.
    pub addons: &'a [String],
    pub trusted: bool,
}

/// The variables a new terminal of `space` starts with: its id, and its active
/// add-ons, whose zsh files are written to the space's own folder under
/// `data_dir`. When they cannot be written the terminal starts without its
/// shell add-ons, and the problem is returned, to be shown.
pub fn terminal_env(
    space: &SpaceTerminal<'_>,
    mac: &Mac,
    zsh: bool,
    data_dir: &Path,
) -> (Vec<(String, String)>, Option<String>) {
    let mut env = vec![("X8AI_SPACE".to_owned(), space.id.to_owned())];
    if space.addons.is_empty() {
        return (env, None);
    }
    let addons: Vec<&Addon> = ADDONS
        .iter()
        .filter(|a| space.addons.iter().any(|id| id == a.id) && active(a, mac, space.trusted, zsh))
        .collect();
    let spaces = data_dir.join("spaces");
    let dir = spaces.join(space.id).join("zsh");
    let setup = terminal_setup(space.title, &addons, &dir, &user_zdotdir(&mac.home), mac);
    if !setup.files.is_empty()
        && let Err(error) = write_files(&spaces, &dir, &setup.files)
    {
        env.extend(
            setup
                .env
                .into_iter()
                .filter(|(name, _)| name != "ZDOTDIR" && name != "X8AI_USER_ZDOTDIR"),
        );
        let problem = format!(
            "This terminal's shell add-ons are off: {} could not be written ({error})",
            dir.display()
        );
        return (env, Some(problem));
    }
    env.extend(setup.env);
    (env, None)
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
