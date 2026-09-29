//! The environment an MCP server starts with (ADR 0017).
//!
//! Not the agent's, and not the whole login environment, which may hold the
//! user's provider keys and other credentials. Exactly:
//!
//! 1. the **base**: the variables every program needs to find its tools and
//!    locale, taken from the login environment (`BASE_VARIABLES`, `LC_*`);
//! 2. the server's **inherited** variables: the ones it lists as `inherit`, from
//!    the login environment, when set there;
//! 3. the server's **secrets**: the ones it lists as `secret`, from the Keychain.
//!
//! Nothing else: no provider key the app configured for the session, no provider
//! variable from the shell, no other server's secret.

use std::fmt;

use x8ai_core::mcp::{McpEnvSource, McpServer};
use x8ai_secrets::{SecretStore, SecretValue};

/// Groups the app's MCP secrets in the Keychain.
pub const KEYCHAIN_SERVICE: &str = "com.x8ai.workspace.mcp";
pub const KEYCHAIN_LABEL: &str = "x8ai Workspace MCP secret";

/// Variables every server gets from the login environment, when set there.
pub const BASE_VARIABLES: &[&str] = &[
    "PATH", "HOME", "USER", "LOGNAME", "SHELL", "TMPDIR", "LANG", "TZ",
];

/// The Keychain account of a server's secret variable.
pub fn secret_account(server: &str, variable: &str) -> String {
    format!("{server}/{variable}")
}

/// A server's environment. May hold secrets: `Debug` shows names only.
#[derive(Clone, Default)]
pub struct ServerEnvironment {
    vars: Vec<(String, String)>,
    /// Values not to show anywhere, even in the server's own error output.
    sensitive: Vec<String>,
}

impl fmt::Debug for ServerEnvironment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("ServerEnvironment")
            .field(
                &self
                    .vars
                    .iter()
                    .map(|(n, _)| n.as_str())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl ServerEnvironment {
    pub fn vars(&self) -> &[(String, String)] {
        &self.vars
    }

    pub fn names(&self) -> Vec<&str> {
        self.vars.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// `text` with every secret or inherited value replaced.
    pub fn redact(&self, text: &str) -> String {
        self.sensitive
            .iter()
            .filter(|v| v.len() >= 4)
            .fold(text.to_owned(), |text, value| {
                text.replace(value.as_str(), "<redacted>")
            })
    }

    fn set(&mut self, name: &str, value: String, sensitive: bool) {
        self.vars.retain(|(n, _)| n != name);
        if sensitive {
            self.sensitive.push(value.clone());
        }
        self.vars.push((name.to_owned(), value));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EnvError {
    #[error("{server}: the secret {name} is not saved; add it in MCP")]
    MissingSecret { server: String, name: String },
    #[error("{server}: {detail}")]
    Store { server: String, detail: String },
}

/// Builds `server`'s environment from the user's `login` environment and its
/// saved secrets. Refused if a secret is missing: a server never starts half
/// configured.
pub fn environment(
    server: &McpServer,
    login: &[(String, String)],
    secrets: &dyn SecretStore,
) -> Result<ServerEnvironment, EnvError> {
    let mut env = ServerEnvironment::default();
    for (name, value) in login {
        if BASE_VARIABLES.contains(&name.as_str()) || name.starts_with("LC_") {
            env.set(name, value.clone(), false);
        }
    }
    for var in &server.env {
        match var.source {
            McpEnvSource::Inherit => {
                if let Some((_, value)) = login.iter().find(|(n, _)| *n == var.name) {
                    env.set(&var.name, value.clone(), true);
                }
            }
            McpEnvSource::Secret => {
                let account = secret_account(server.id.as_str(), &var.name);
                let value: SecretValue = secrets
                    .get(&account)
                    .map_err(|e| EnvError::Store {
                        server: server.name.clone(),
                        detail: e.to_string(),
                    })?
                    .ok_or_else(|| EnvError::MissingSecret {
                        server: server.name.clone(),
                        name: var.name.clone(),
                    })?;
                env.set(&var.name, value.expose().to_owned(), true);
            }
        }
    }
    Ok(env)
}
