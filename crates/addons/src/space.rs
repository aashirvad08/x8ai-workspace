//! A space's terminals with its add-ons, the same in the app and in `x8ai`:
//! which added add-ons are on, the variables a terminal starts with, the zsh
//! files written for them in the space's own folder, and stand-ins for the
//! programs turned off in it (ADR 0019).

use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use x8ai_core::addon::AddonReach;

use crate::mac::{Mac, quote};
use crate::registry::{ADDONS, Addon, Need};
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
    /// Programs for the whole Mac turned off in it, by add-on id.
    pub off: &'a [String],
    pub trusted: bool,
}

/// Writes a stand-in for each program turned off in `space` to its own folder
/// under `data_dir`, and removes those turned on again. Returns the folder when
/// there are any: first on the `PATH`, it hides the real programs. A stand-in
/// says what is off and where, and fails as a missing program does (127).
pub fn off_programs(
    space: &SpaceTerminal<'_>,
    data_dir: &Path,
) -> std::io::Result<Option<PathBuf>> {
    let spaces = data_dir.join("spaces");
    let dir = spaces.join(space.id).join("off");
    let stand_ins: Vec<(&'static str, String)> = ADDONS
        .iter()
        .filter(|a| a.reach == AddonReach::Mac && space.off.iter().any(|id| id == a.id))
        .filter_map(|addon| {
            let Need::Program(program) = addon.need else {
                return None;
            };
            let message = format!(
                "{program}: {} is off in {}. x8ai's Add-ons turn it back on.",
                addon.name, space.title
            );
            let text = format!(
                "#!/bin/sh\n# Written by x8ai Workspace: {} is off in this space.\nprintf '%s\\n' {} >&2\nexit 127\n",
                addon.name,
                quote(&message)
            );
            Some((program, text))
        })
        .collect();
    if stand_ins.is_empty() {
        match std::fs::remove_dir_all(&dir) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error),
            _ => return Ok(None),
        }
    }
    write_files(&spaces, &dir, &stand_ins, 0o700)?;
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let name = entry.file_name();
        if !stand_ins.iter().any(|(program, _)| name == *program) {
            std::fs::remove_file(entry.path())?;
        }
    }
    Ok(Some(dir))
}

/// The variables a new terminal of `space` starts with: its id, and its active
/// add-ons, whose zsh files are written to the space's own folder under
/// `data_dir`, as are the stand-ins of the programs turned off in it (zsh
/// only: its files put them first). When they cannot be written the terminal
/// starts without them, and the problem is returned, to be shown.
pub fn terminal_env(
    space: &SpaceTerminal<'_>,
    mac: &Mac,
    zsh: bool,
    data_dir: &Path,
) -> (Vec<(String, String)>, Option<String>) {
    let mut env = vec![("X8AI_SPACE".to_owned(), space.id.to_owned())];
    if space.addons.is_empty() && space.off.is_empty() {
        return (env, None);
    }
    let (off, problem) = match off_programs(space, data_dir) {
        Ok(off) if zsh => (off, None),
        Ok(_) => (None, None),
        Err(error) => (
            None,
            Some(format!(
                "Programs turned off in this space are on in this terminal: their stand-ins could not be written ({error})"
            )),
        ),
    };
    let addons: Vec<&Addon> = ADDONS
        .iter()
        .filter(|a| space.addons.iter().any(|id| id == a.id) && active(a, mac, space.trusted, zsh))
        .collect();
    let spaces = data_dir.join("spaces");
    let dir = spaces.join(space.id).join("zsh");
    let setup = terminal_setup(
        space.title,
        &addons,
        &dir,
        &user_zdotdir(&mac.home),
        mac,
        off.as_deref(),
    );
    if !setup.files.is_empty()
        && let Err(error) = write_files(&spaces, &dir, &setup.files, 0o600)
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
    (env, problem)
}

/// Writes the space's files (its zsh files, its stand-ins), the user's alone
/// (`mode`), each replaced whole.
fn write_files(
    spaces: &Path,
    dir: &Path,
    files: &[(&'static str, String)],
    mode: u32,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(spaces, std::fs::Permissions::from_mode(0o700))?;
    for (name, text) in files {
        let temp = dir.join(format!("{name}.tmp-{}", std::process::id()));
        let written = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(mode)
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

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn a_program_turned_off_has_a_stand_in_that_says_so() {
        let data = tempfile::tempdir().unwrap();
        // Starship is not a program for the whole Mac: off means nothing there.
        let off = ["neovim".to_owned(), "starship".to_owned()];
        let space = SpaceTerminal {
            id: "ws-abcdef",
            title: "Bob's app (ws-abcdef)",
            addons: &[],
            off: &off,
            trusted: true,
        };
        let dir = off_programs(&space, data.path()).unwrap().unwrap();
        let names: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(names, ["nvim"]);
        let output = Command::new(dir.join("nvim"))
            .arg("notes.txt")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(127));
        assert_eq!(
            String::from_utf8_lossy(&output.stderr),
            "nvim: Neovim is off in Bob's app (ws-abcdef). x8ai's Add-ons turn it back on.\n"
        );

        // On again: the stand-in and its folder are gone.
        let space = SpaceTerminal { off: &[], ..space };
        assert_eq!(off_programs(&space, data.path()).unwrap(), None);
        assert!(!dir.exists());
    }
}
