use std::ffi::OsString;
use std::path::PathBuf;

use portable_pty::CommandBuilder;

use crate::locale;

/// Claude Code sets this in the environment of every process it starts, and a
/// Claude Code that finds it takes itself for that session's child: it saves no
/// transcript ("Transcript saving is off — inherited CLAUDE_CODE_CHILD_SESSION
/// marker"). It reaches the app when the app itself was started from inside a
/// Claude Code session (`pnpm tauri dev` or `open` run there: macOS gives an
/// opened app its caller's environment). A terminal the app opens is not part of
/// that session, so no session, shell or agent, gets the marker: `claude` typed
/// in one starts as a session of its own. Nothing else is removed.
pub const CLAUDE_CODE_CHILD_SESSION: &str = "CLAUDE_CODE_CHILD_SESSION";

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
    /// defaults to the home directory. Used by the agent runtime and by tests.
    Exec {
        program: PathBuf,
        args: Vec<OsString>,
        cwd: Option<PathBuf>,
        env: Environment,
    },
}

/// The environment a program starts with, before the terminal variables every
/// session gets (`TERM`, `COLORTERM`, `TERM_PROGRAM`, and `LANG` if unset).
#[derive(Clone, Default)]
pub enum Environment {
    /// The app's own environment.
    #[default]
    Inherit,
    /// Exactly these variables, and nothing from the app's environment.
    Exactly(Vec<(String, String)>),
}

/// Names only: values can be credentials (an agent's API key).
impl std::fmt::Debug for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Inherit => f.write_str("Inherit"),
            Self::Exactly(vars) => f
                .debug_tuple("Exactly")
                .field(
                    &vars
                        .iter()
                        .map(|(name, _)| name.as_str())
                        .collect::<Vec<_>>(),
                )
                .finish(),
        }
    }
}

/// The user's default shell: `$SHELL` if it is executable, otherwise the shell in
/// their account record, otherwise `/bin/sh`. The one [`Program::LoginShell`] runs.
pub fn user_shell() -> String {
    CommandBuilder::new_default_prog().get_shell()
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
            Self::Exec {
                program,
                args,
                cwd,
                env,
            } => {
                let mut cmd = CommandBuilder::new(program);
                cmd.args(args);
                if let Environment::Exactly(vars) = env {
                    cmd.env_clear();
                    for (name, value) in vars {
                        cmd.env(name, value);
                    }
                }
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
        cmd.env_remove(CLAUDE_CODE_CHILD_SESSION);
        (cmd, path, cwd)
    }
}

fn home() -> PathBuf {
    std::env::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_environment_prints_names_but_never_values() {
        let env = Environment::Exactly(vec![("API_KEY".into(), "sk-test-invalid".into())]);
        let program = Program::Exec {
            program: "/bin/true".into(),
            args: Vec::new(),
            cwd: None,
            env,
        };
        let printed = format!("{program:?}");
        assert!(
            printed.contains("API_KEY") && !printed.contains("sk-test"),
            "{printed}"
        );
    }
}
