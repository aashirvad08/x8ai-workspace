//! Turning add-ons on in one space's terminals only.
//!
//! A space's terminal is the user's login zsh with `ZDOTDIR` set to a folder of
//! the space's own. zsh reads its startup files from there; each reads the
//! user's own file first, exactly where zsh would have found it (their
//! `ZDOTDIR`, or home), and the last one hands `ZDOTDIR` back. After the user's
//! `.zshrc`, the space's `.zshrc` turns on its add-ons. Programs turned off in
//! the space have stand-ins in a folder put first on the `PATH`, after the
//! user's `.zshrc` and `.zlogin`, so their own `PATH` lines cannot put the real
//! ones back in front. The user's files are never written, and terminals of
//! other spaces, and other apps', are unchanged.

use std::path::Path;

use crate::mac::{Mac, quote};
use crate::registry::{Addon, Need};

/// What a space's new terminals start with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalSetup {
    /// zsh startup files to write in the space's own folder, by name. Empty when
    /// no add-on changes the shell; then `ZDOTDIR` is not set either.
    pub files: Vec<(&'static str, String)>,
    pub env: Vec<(String, String)>,
}

/// The setup for a space with these add-ons (the active ones only). `dir` is the
/// space's own folder for zsh's files, `user_zdotdir` where the user's own are,
/// and `off` the folder of stand-ins for the programs turned off in it, if any.
pub fn terminal_setup(
    title: &str,
    addons: &[&Addon],
    dir: &Path,
    user_zdotdir: &Path,
    mac: &Mac,
    off: Option<&Path>,
) -> TerminalSetup {
    let mut env: Vec<(String, String)> = addons
        .iter()
        .flat_map(|a| a.env)
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    let lines: Vec<String> = addons
        .iter()
        .filter_map(|addon| {
            let zshrc = addon.zshrc?;
            let line = match addon.need {
                Need::BrewFile(file) => zshrc.replace(
                    "{file}",
                    &quote(&mac.brew_file(file)?.display().to_string()),
                ),
                _ => zshrc.to_owned(),
            };
            Some(format!("# {}\n{line}", addon.name))
        })
        .collect();
    if lines.is_empty() && off.is_none() {
        return TerminalSetup {
            files: Vec::new(),
            env,
        };
    }
    // The stand-ins first, the folder once: `${path:#…}` drops it where it was.
    let first_on_path = off.map_or_else(String::new, |off| {
        let off = quote(&off.display().to_string());
        format!("\n# Programs turned off here: their stand-ins come first.\npath=({off} ${{path:#{off}}})\n")
    });
    env.push(("ZDOTDIR".to_owned(), dir.display().to_string()));
    env.push((
        "X8AI_USER_ZDOTDIR".to_owned(),
        user_zdotdir.display().to_string(),
    ));
    let header = format!(
        "# Written by x8ai Workspace for {title}, and rewritten for every new terminal\n\
         # there: change your own files instead. Your own file runs here, as it always does.\n"
    );
    let reads = |name: &str| {
        format!(
            "ZDOTDIR=$X8AI_USER_ZDOTDIR\n\
             [[ -r \"$ZDOTDIR/{name}\" ]] && builtin source \"$ZDOTDIR/{name}\"\n\
             X8AI_USER_ZDOTDIR=$ZDOTDIR\n"
        )
    };
    let zshenv = format!(
        "{header}\n\
         _x8ai_zdotdir=$ZDOTDIR\n\
         X8AI_USER_ZDOTDIR=${{X8AI_USER_ZDOTDIR:-$HOME}}\n\
         # Hands ZDOTDIR back to yours once the last startup file has run, so a zsh\n\
         # started inside reads only your own files.\n\
         _x8ai_done() {{\n\
         \x20 if [[ $X8AI_USER_ZDOTDIR == $HOME ]]; then unset ZDOTDIR; else export ZDOTDIR=$X8AI_USER_ZDOTDIR; fi\n\
         \x20 unset X8AI_USER_ZDOTDIR _x8ai_zdotdir\n\
         \x20 unfunction _x8ai_done\n\
         }}\n\
         if [[ $X8AI_USER_ZDOTDIR != $_x8ai_zdotdir ]]; then\n\
         {reads}\
         fi\n\
         ZDOTDIR=$_x8ai_zdotdir\n",
        reads = reads(".zshenv")
    );
    let zprofile = format!(
        "{header}\n{reads}ZDOTDIR=$_x8ai_zdotdir\n",
        reads = reads(".zprofile")
    );
    let zshrc = format!(
        "{header}\n\
         # /etc/zshrc put the history in this folder; it belongs with yours.\n\
         HISTFILE=$X8AI_USER_ZDOTDIR/.zsh_history\n\
         {reads}\
         ZDOTDIR=$_x8ai_zdotdir\n\
         \n\
         # The add-ons of {title}\n\
         {addons}\n\
         {first_on_path}\
         \n\
         # A login shell reads .zlogin next, which finishes; any other is done here.\n\
         [[ -o login ]] || _x8ai_done\n",
        reads = reads(".zshrc"),
        addons = lines.join("\n"),
    );
    let zlogin = format!(
        "{header}\n{reads}{first_on_path}_x8ai_done\n",
        reads = reads(".zlogin")
    );
    TerminalSetup {
        files: vec![
            (".zshenv", zshenv),
            (".zprofile", zprofile),
            (".zshrc", zshrc),
            (".zlogin", zlogin),
        ],
        env,
    }
}

/// Whether the user's own startup files (their text, joined) already turn the
/// add-on on: a marker on a line that is not a comment.
pub fn in_your_shell(addon: &Addon, startup: &str) -> bool {
    !addon.markers.is_empty()
        && startup.lines().any(|line| {
            let line = line.trim_start();
            !line.starts_with('#') && addon.markers.iter().any(|m| line.contains(m))
        })
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
    use std::process::Command;

    use super::*;
    use crate::registry::find;

    fn mac(prefix: &Path) -> Mac {
        let bin = prefix.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let brew = bin.join("brew");
        fs::write(&brew, "").unwrap();
        fs::set_permissions(&brew, fs::Permissions::from_mode(0o755)).unwrap();
        Mac {
            home: "/Users/me".into(),
            path: Some(bin.display().to_string()),
        }
    }

    #[test]
    fn no_shell_add_on_leaves_the_shell_alone() {
        let dir = tempfile::tempdir().unwrap();
        let setup = terminal_setup(
            "gymRL",
            &[find("lazyvim").unwrap(), find("ripgrep").unwrap()],
            dir.path(),
            Path::new("/Users/me"),
            &mac(dir.path()),
            None,
        );
        assert!(setup.files.is_empty());
        assert_eq!(
            setup.env,
            [("NVIM_APPNAME".to_owned(), "x8ai-lazyvim".to_owned())]
        );
    }

    #[test]
    fn shell_add_ons_come_after_the_users_own_zshrc_with_paths_quoted() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("home brew");
        let setup = terminal_setup(
            "gymRL",
            &[
                find("starship").unwrap(),
                find("syntax-highlighting").unwrap(),
            ],
            Path::new("/data/spaces/ws-abcdef/zsh"),
            Path::new("/Users/me"),
            &mac(&prefix),
            None,
        );
        let zshrc = &setup.files.iter().find(|(n, _)| *n == ".zshrc").unwrap().1;
        let own = zshrc.find("builtin source \"$ZDOTDIR/.zshrc\"").unwrap();
        let starship = zshrc.find("starship init zsh").unwrap();
        let highlighting = zshrc
            .find(&format!(
                "'{}/share/zsh-syntax-highlighting/zsh-syntax-highlighting.zsh'",
                prefix.display()
            ))
            .unwrap();
        assert!(own < starship && starship < highlighting, "{zshrc}");
        assert!(setup.env.contains(&(
            "ZDOTDIR".to_owned(),
            "/data/spaces/ws-abcdef/zsh".to_owned()
        )));
    }

    #[test]
    fn markers_count_only_outside_comments() {
        let starship = find("starship").unwrap();
        assert!(in_your_shell(
            starship,
            "export A=1\neval \"$(starship init zsh)\"\n"
        ));
        assert!(!in_your_shell(
            starship,
            "  # eval \"$(starship init zsh)\"\n"
        ));
        assert!(!in_your_shell(find("ripgrep").unwrap(), "rg"));
    }

    /// Runs the files with the real zsh: the user's own files run in order, the
    /// add-on line runs after their `.zshrc`, the history stays theirs, and
    /// `ZDOTDIR` is theirs again at the end.
    #[test]
    fn zsh_reads_the_users_files_then_the_add_ons() {
        let zsh = PathBuf::from("/bin/zsh");
        if !zsh.is_file() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let space = dir.path().join("space");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&space).unwrap();
        for name in [".zshenv", ".zprofile", ".zshrc", ".zlogin"] {
            fs::write(home.join(name), format!("order+=({name})\n")).unwrap();
        }
        // A stand-in add-on: a zshrc line that records itself.
        let addon = Addon {
            zshrc: Some("order+=(add-on)"),
            ..*find("fzf").unwrap()
        };
        let setup = terminal_setup(
            "test",
            &[&addon],
            &space,
            &home,
            &Mac {
                home: home.clone(),
                path: None,
            },
            None,
        );
        for (name, text) in &setup.files {
            fs::write(space.join(name), text).unwrap();
        }
        let output = Command::new(&zsh)
            .args([
                "-l",
                "-i",
                "-c",
                "print -r -- \"$order|$HISTFILE|${ZDOTDIR-unset}\"",
            ])
            .env_clear()
            .env("HOME", &home)
            .env("TERM", "dumb")
            .envs(setup.env.iter().cloned())
            .output()
            .unwrap();
        let printed = String::from_utf8_lossy(&output.stdout);
        let last = printed.lines().last().unwrap_or_default();
        assert_eq!(
            last,
            format!(
                ".zshenv .zprofile .zshrc add-on .zlogin|{}/.zsh_history|unset",
                home.display()
            ),
            "{printed}{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// The real zsh again: a program turned off in the space is its stand-in,
    /// even when the user's `.zprofile` and `.zlogin` put the folder of the
    /// real one first, as Homebrew's line does; and the stand-ins' folder is
    /// on the `PATH` once.
    #[test]
    fn zsh_finds_the_stand_in_of_a_program_turned_off() {
        let zsh = PathBuf::from("/bin/zsh");
        if !zsh.is_file() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let space = dir.path().join("space");
        let brew = dir.path().join("brew bin");
        let off = dir.path().join("off folder");
        for folder in [&home, &space, &brew, &off] {
            fs::create_dir_all(folder).unwrap();
        }
        let program = |path: PathBuf, says: &str| {
            fs::write(&path, format!("#!/bin/sh\necho {says}\n")).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        };
        program(brew.join("nvim"), "REAL");
        program(off.join("nvim"), "STAND-IN");
        let first = format!("path=('{}' $path)\n", brew.display());
        fs::write(home.join(".zprofile"), &first).unwrap();
        fs::write(home.join(".zlogin"), &first).unwrap();
        let setup = terminal_setup(
            "test",
            &[],
            &space,
            &home,
            &Mac {
                home: home.clone(),
                path: None,
            },
            Some(&off),
        );
        assert!(!setup.files.is_empty());
        for (name, text) in &setup.files {
            fs::write(space.join(name), text).unwrap();
        }
        let output = Command::new(&zsh)
            .args([
                "-l",
                "-i",
                "-c",
                "found=(${(M)path:#*off folder}); print -r -- \"$(nvim)|$#found\"",
            ])
            .env_clear()
            .env("HOME", &home)
            .env("TERM", "dumb")
            .env("PATH", "/usr/bin:/bin")
            .envs(setup.env.iter().cloned())
            .output()
            .unwrap();
        let printed = String::from_utf8_lossy(&output.stdout);
        assert_eq!(
            printed.lines().last().unwrap_or_default(),
            "STAND-IN|1",
            "{printed}{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
