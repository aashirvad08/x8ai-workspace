use std::ffi::OsString;
use std::path::PathBuf;

use portable_pty::CommandBuilder;

use crate::locale;

/// What a session runs.
#[derive(Debug, Clone)]
pub enum Program {
    /// The user's default shell, started as a login shell, as Terminal.app does.
    /// The shell is `$SHELL` if it is executable, otherwise the shell in the user's
    /// account record, otherwise `/bin/sh`. A login shell reads the user's profile,
    /// so `PATH` matches their normal terminal even though a GUI app inherits a
    /// minimal environment. `cwd` defaults to the home directory.
    LoginShell { cwd: Option<PathBuf> },
    /// A specific executable with arguments, never interpreted by a shell. `cwd`
    /// defaults to the home directory. Used by tests today and by the agent runtime
    /// in Phase 4.
    Exec {
        program: PathBuf,
        args: Vec<OsString>,
        cwd: Option<PathBuf>,
    },
}

impl Program {
    /// The command to spawn, the absolute path of the program it runs, and the
    /// directory it starts in.
    pub(crate) fn command(&self) -> (CommandBuilder, String, PathBuf) {
        let (mut cmd, path, cwd) = match self {
            Self::LoginShell { cwd } => {
                // portable-pty resolves the shell and prefixes argv[0] with `-`, the
                // login-shell convention.
                let cmd = CommandBuilder::new_default_prog();
                let shell = cmd.get_shell();
                (cmd, shell, cwd.clone().unwrap_or_else(home))
            }
            Self::Exec { program, args, cwd } => {
                let mut cmd = CommandBuilder::new(program);
                cmd.args(args);
                (
                    cmd,
                    program.display().to_string(),
                    cwd.clone().unwrap_or_else(home),
                )
            }
        };
        cmd.cwd(&cwd);

        // The environment is inherited from the app. These are the variables every
        // terminal emulator sets so programs know what they are talking to.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "x8ai-workspace");
        cmd.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
        let names_a_locale = ["LANG", "LC_ALL", "LC_CTYPE"]
            .iter()
            .any(|key| cmd.get_env(key).is_some_and(|v| !v.is_empty()));
        if !names_a_locale && let Some(lang) = locale::user_lang() {
            cmd.env("LANG", lang);
        }
        (cmd, path, cwd)
    }
}

fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}
