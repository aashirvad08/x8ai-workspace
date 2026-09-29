//! MCP servers: GitHub, Playwright, filesystem, databases and others.
//!
//! In the initial design the app is not the MCP client: the agent is, and the app
//! supplies it with server configuration. A local (stdio) MCP server is a program
//! running with the user's privileges, so it is described by the same explicit
//! [`LaunchSpec`] as an agent and is subject to the same approval rules.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::definition::{DefinitionError, check_endpoint_url, check_name};
use crate::id::IntegrationId;
use crate::launch::{LaunchSpec, Requirement};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct McpServerDefinition {
    pub id: IntegrationId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub transport: McpTransport,
    #[serde(default)]
    pub requirements: Vec<Requirement>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub enum McpTransport {
    /// A local process speaking MCP over stdin/stdout.
    Stdio { launch: LaunchSpec },
    /// A remote server speaking MCP Streamable HTTP. Authentication (OAuth, per the
    /// MCP specification) is designed in Phase 7.
    StreamableHttp { url: String },
}

/// The transport kinds, used by agents to declare what they can connect to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum McpTransportKind {
    Stdio,
    StreamableHttp,
}

impl McpServerDefinition {
    pub fn validate(&self) -> Result<(), DefinitionError> {
        check_name(&self.name)?;
        match &self.transport {
            McpTransport::Stdio { launch } => launch.validate("transport.launch")?,
            // Tool calls carry workspace content, so remote servers get the same
            // transport rule as credentials.
            McpTransport::StreamableHttp { url } => {
                check_endpoint_url("transport.url", url, true)?;
            }
        }
        for (i, requirement) in self.requirements.iter().enumerate() {
            requirement.validate(&format!("requirements[{i}]"))?;
        }
        Ok(())
    }
}

// The MCP server registry (Phase 7, docs/mcp.md). What the user configured in the
// app, as stored in `mcp-servers.json` and shown in the webview. None of these
// types can hold a secret value: a variable is named, and says where its value
// comes from.

/// An MCP server the user added in the app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct McpServer {
    /// Made natively from the name when the server is added; stable afterwards.
    pub id: IntegrationId,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub transport: McpServerTransport,
    /// Variables the server needs, by name. Only for stdio servers.
    #[serde(default)]
    pub env: Vec<McpEnvVar>,
    /// A disabled server is never attached to a new session, nor started for an
    /// existing one.
    pub enabled: bool,
    pub scope: McpScope,
}

/// How the server is reached. Never a shell command line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
#[ts(export)]
pub enum McpServerTransport {
    /// A local program speaking MCP over stdin and stdout, started by the app
    /// without a shell: `command` (an absolute path, or a name looked up on the
    /// user's login `PATH`) and its arguments, each passed as it is.
    Stdio { command: String, args: Vec<String> },
    /// A server at a URL speaking MCP Streamable HTTP. The agent connects to it;
    /// the app never does.
    StreamableHttp { url: String },
}

impl McpServerTransport {
    pub fn kind(&self) -> McpTransportKind {
        match self {
            Self::Stdio { .. } => McpTransportKind::Stdio,
            Self::StreamableHttp { .. } => McpTransportKind::StreamableHttp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct McpEnvVar {
    pub name: String,
    pub source: McpEnvSource,
}

/// Where a server's variable gets its value. There is no literal value: a value
/// typed into the registry could be a secret, and the registry is plain JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum McpEnvSource {
    /// Saved in the macOS Keychain for this server.
    Secret,
    /// The variable of the same name in the user's login environment, if set.
    Inherit,
}

/// Which sessions a server is attached to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub enum McpScope {
    /// Every new agent session, in any workspace.
    Global,
    /// New agent sessions in this workspace (its absolute root).
    Workspace { root: String },
    /// Only sessions it is chosen for at launch.
    Session,
}

/// A server as the webview adds or changes it. The id, and the folder of a
/// workspace scope (the open workspace), are decided natively.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[ts(export)]
pub struct McpServerInput {
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub transport: McpServerTransport,
    #[serde(default)]
    pub env: Vec<McpEnvVar>,
    pub enabled: bool,
    pub scope: McpScopeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum McpScopeKind {
    Global,
    Workspace,
    Session,
}

/// Shells run their arguments as commands; a stdio server is never started
/// through one.
const SHELLS: &[&str] = &[
    "sh", "bash", "zsh", "dash", "ksh", "fish", "csh", "tcsh", "mksh",
];
/// Characters a shell would interpret. A bare command name containing one is a
/// command line, not a program name.
const SHELL_CHARACTERS: &str = " \t;&|<>()$`\\\"'*?[]{}~!#=%";
/// Prefixes of well-known credentials. An argument or URL holding one belongs in
/// a secret variable, not in the plain-text registry.
const CREDENTIAL_PREFIXES: &[&str] = &[
    "sk-",
    "sk_live_",
    "ghp_",
    "gho_",
    "ghs_",
    "ghu_",
    "github_pat_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "AKIA",
    "ASIA",
    "AIza",
    "ya29.",
];

impl McpServerInput {
    pub fn validate(&self) -> Result<(), DefinitionError> {
        check_name(&self.name)?;
        validate_transport(&self.transport)?;
        validate_env(&self.transport, &self.env)
    }
}

impl McpServer {
    pub fn validate(&self) -> Result<(), DefinitionError> {
        check_name(&self.name)?;
        validate_transport(&self.transport)?;
        validate_env(&self.transport, &self.env)?;
        if let McpScope::Workspace { root } = &self.scope
            && !root.starts_with('/')
        {
            return Err(DefinitionError::new(
                "scope.root",
                "must be an absolute path",
            ));
        }
        Ok(())
    }

    /// The names of the variables whose values are saved secrets.
    pub fn secret_names(&self) -> impl Iterator<Item = &str> {
        self.env
            .iter()
            .filter(|v| v.source == McpEnvSource::Secret)
            .map(|v| v.name.as_str())
    }
}

fn validate_transport(transport: &McpServerTransport) -> Result<(), DefinitionError> {
    match transport {
        McpServerTransport::Stdio { command, args } => {
            check_command(command)?;
            for (i, arg) in args.iter().enumerate() {
                let field = format!("transport.args[{i}]");
                if arg.chars().any(|c| c.is_control()) {
                    return Err(DefinitionError::new(
                        field,
                        "must not contain control characters",
                    ));
                }
                if looks_like_credential(arg) {
                    return Err(DefinitionError::new(
                        field,
                        "looks like a credential; add it as a secret variable instead",
                    ));
                }
            }
            Ok(())
        }
        McpServerTransport::StreamableHttp { url } => {
            // Tool calls carry workspace content: https, unless on this machine.
            check_endpoint_url("transport.url", url, true)?;
            // Agents expand `${VAR}` (Claude Code) and `{env:VAR}` (OpenCode) in
            // server URLs; the app hands them a URL that expands to nothing.
            if url.contains(['$', '{', '}']) {
                return Err(DefinitionError::new(
                    "transport.url",
                    "must not contain $, { or }: agents would fill in values from their environment",
                ));
            }
            let parsed = url::Url::parse(url).expect("checked above");
            if parsed.fragment().is_some() {
                return Err(DefinitionError::new(
                    "transport.url",
                    "must not have a fragment",
                ));
            }
            let credential_in_query = parsed.query_pairs().any(|(name, value)| {
                let name = name.to_ascii_lowercase();
                ["key", "token", "secret", "password", "auth"]
                    .iter()
                    .any(|word| name.contains(word))
                    || looks_like_credential(&value)
            });
            if credential_in_query || looks_like_credential(parsed.path()) {
                return Err(DefinitionError::new(
                    "transport.url",
                    "must not carry a credential; the agent authenticates to the server itself",
                ));
            }
            Ok(())
        }
    }
}

/// An absolute path, or a plain program name to look up on the login `PATH`.
/// Never a relative path (it would resolve inside the workspace, which is the
/// repository's code), never a command line, never a shell.
fn check_command(command: &str) -> Result<(), DefinitionError> {
    let field = "transport.command";
    if command.trim().is_empty() {
        return Err(DefinitionError::new(field, "must not be empty"));
    }
    if command.chars().any(|c| c.is_control()) {
        return Err(DefinitionError::new(
            field,
            "must not contain control characters",
        ));
    }
    if command.len() > 1024 {
        return Err(DefinitionError::new(field, "is too long"));
    }
    if command.starts_with('/') {
        if command.split('/').any(|part| part == ".." || part == ".") {
            return Err(DefinitionError::new(
                field,
                "must not contain . or .. segments",
            ));
        }
    } else if command.contains('/') {
        return Err(DefinitionError::new(
            field,
            "must be an absolute path or a program name on your PATH, not a relative path",
        ));
    } else if command.chars().any(|c| SHELL_CHARACTERS.contains(c)) {
        return Err(DefinitionError::new(
            field,
            "must be one program name; put its arguments in the argument list",
        ));
    }
    let program = command.rsplit('/').next().unwrap_or(command);
    if SHELLS.contains(&program) {
        return Err(DefinitionError::new(
            field,
            "must not be a shell; MCP servers are started directly, never through a shell",
        ));
    }
    Ok(())
}

fn validate_env(transport: &McpServerTransport, env: &[McpEnvVar]) -> Result<(), DefinitionError> {
    if matches!(transport, McpServerTransport::StreamableHttp { .. }) && !env.is_empty() {
        return Err(DefinitionError::new(
            "env",
            "applies to stdio servers only; an HTTP server gets nothing from this machine",
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for (i, var) in env.iter().enumerate() {
        let field = format!("env[{i}].name");
        if !crate::id::is_env_var_name(&var.name) {
            return Err(DefinitionError::new(field, crate::id::ENV_VAR_NAME_RULE));
        }
        if !seen.insert(var.name.as_str()) {
            return Err(DefinitionError::new(
                field,
                format!("{} is listed more than once", var.name),
            ));
        }
    }
    Ok(())
}

fn looks_like_credential(value: &str) -> bool {
    let value = value.rsplit_once('=').map_or(value, |(_, v)| v);
    let value = value.trim_start_matches('/');
    CREDENTIAL_PREFIXES
        .iter()
        .any(|prefix| value.starts_with(prefix) && value.len() >= prefix.len() + 8)
}

// IPC contracts of the MCP layer.

/// A server, and what it still needs. Returned by `mcp_list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpServerStatus {
    pub server: McpServer,
    /// Each secret variable, and whether its value is saved. Never the value.
    pub secrets: Vec<McpSecretStatus>,
    /// Every secret saved, and for stdio, the command found.
    pub configured: bool,
    /// Why it cannot run as configured, if it cannot.
    pub problem: Option<String>,
    /// Which agents can use it, as their adapters say.
    pub agents: Vec<McpAgentSupport>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpSecretStatus {
    pub name: String,
    pub state: crate::model::CredentialState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpAgentSupport {
    pub agent: IntegrationId,
    pub supported: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct McpServerList {
    pub servers: Vec<McpServerStatus>,
}

/// An MCP server attached to an agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct SessionMcpServer {
    pub id: IntegrationId,
    pub name: String,
    pub transport: Option<McpTransportKind>,
    pub state: McpServerState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "state", rename_all = "camelCase")]
#[ts(export)]
pub enum McpServerState {
    /// The agent is not running; nothing is started.
    Idle,
    /// A server at a URL: the agent connects to it itself.
    Remote,
    /// Ready for the agent; started when the agent connects.
    Waiting,
    Running {
        pid: u32,
    },
    /// It ended; with its exit code, if it exited normally.
    Exited {
        code: Option<i32>,
    },
    Failed {
        message: String,
    },
    /// Not used by this session's current run, and why (disabled, removed…).
    Skipped {
        reason: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stdio(command: &str, args: &[&str]) -> McpServerInput {
        McpServerInput {
            name: "Test".into(),
            description: String::new(),
            transport: McpServerTransport::Stdio {
                command: command.into(),
                args: args.iter().map(|a| (*a).to_owned()).collect(),
            },
            env: Vec::new(),
            enabled: true,
            scope: McpScopeKind::Global,
        }
    }

    fn http(url: &str) -> McpServerInput {
        McpServerInput {
            transport: McpServerTransport::StreamableHttp { url: url.into() },
            ..stdio("x", &[])
        }
    }

    #[test]
    fn a_stdio_command_is_one_program_never_a_command_line() {
        for good in [
            "npx",
            "uvx",
            "python3.12",
            "/opt/homebrew/bin/npx",
            "/Applications/My App.app/Contents/MacOS/server",
        ] {
            assert!(stdio(good, &[]).validate().is_ok(), "{good}");
        }
        for bad in [
            "",
            "  ",
            "npx -y @modelcontextprotocol/server-github",
            "npx;rm -rf ~",
            "$(curl evil)",
            "`id`",
            "server|tee",
            "./server",
            "bin/server",
            "../server",
            "/usr/bin/../bin/server",
            "sh",
            "/bin/sh",
            "/bin/bash",
            "zsh",
            "a\nb",
        ] {
            assert!(stdio(bad, &[]).validate().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn arguments_are_structured_and_passed_as_they_are() {
        // A shell would interpret these; the app passes each as one argument.
        let input = stdio(
            "npx",
            &[
                "-y",
                "@scope/server@1.0.0",
                "; rm -rf ~",
                "$(id)",
                "--root",
                "/tmp/a b",
            ],
        );
        assert!(input.validate().is_ok());
        assert!(stdio("npx", &["line\nbreak"]).validate().is_err());
        assert!(stdio("npx", &["nul\0byte"]).validate().is_err());
    }

    #[test]
    fn a_credential_in_the_plain_text_registry_is_refused() {
        let arg = format!("--token=ghp_{}", "a".repeat(36));
        assert!(stdio("npx", &[&arg]).validate().is_err());
        assert!(
            stdio("npx", &[&format!("sk-{}", "b".repeat(40))])
                .validate()
                .is_err()
        );
        assert!(
            http("https://mcp.example.com/mcp?api_key=abc")
                .validate()
                .is_err()
        );
        assert!(
            http("https://user:pass@mcp.example.com/mcp")
                .validate()
                .is_err()
        );
    }

    #[test]
    fn http_servers_need_a_valid_url_and_https_away_from_this_machine() {
        assert!(http("https://mcp.example.com/mcp").validate().is_ok());
        assert!(http("http://127.0.0.1:8931/mcp").validate().is_ok());
        assert!(http("http://localhost:8931/mcp").validate().is_ok());
        for bad in [
            "",
            "mcp.example.com",
            "http://mcp.example.com/mcp",
            "file:///etc/passwd",
            "ftp://x/y",
            "https://x/y#frag",
            "javascript:alert(1)",
            "https://evil.example/${GITHUB_TOKEN}",
            "https://evil.example/{env:GITHUB_TOKEN}",
        ] {
            assert!(http(bad).validate().is_err(), "{bad}");
        }
    }

    #[test]
    fn variables_are_names_with_a_source_and_only_for_stdio() {
        let var = |name: &str, source| McpEnvVar {
            name: name.into(),
            source,
        };
        let mut input = stdio("npx", &[]);
        input.env = vec![
            var("GITHUB_PERSONAL_ACCESS_TOKEN", McpEnvSource::Secret),
            var("HTTPS_PROXY", McpEnvSource::Inherit),
        ];
        assert!(input.validate().is_ok());
        input.env.push(var("NOT-VALID", McpEnvSource::Secret));
        assert!(input.validate().is_err());
        input.env.pop();
        input.env.push(var("HTTPS_PROXY", McpEnvSource::Secret));
        assert!(input.validate().is_err(), "duplicate");
        let mut remote = http("https://mcp.example.com/mcp");
        remote.env = vec![var("TOKEN", McpEnvSource::Secret)];
        assert!(remote.validate().is_err());
    }

    #[test]
    fn a_registry_entry_serializes_without_any_value() {
        let server = McpServer {
            id: IntegrationId::new("github").unwrap(),
            name: "GitHub".into(),
            description: String::new(),
            transport: McpServerTransport::Stdio {
                command: "npx".into(),
                args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
            },
            env: vec![McpEnvVar {
                name: "GITHUB_PERSONAL_ACCESS_TOKEN".into(),
                source: McpEnvSource::Secret,
            }],
            enabled: true,
            scope: McpScope::Workspace {
                root: "/Users/me/project".into(),
            },
        };
        assert!(server.validate().is_ok());
        let json = serde_json::to_value(&server).unwrap();
        assert_eq!(
            json["env"],
            serde_json::json!([{ "name": "GITHUB_PERSONAL_ACCESS_TOKEN", "source": "secret" }])
        );
        assert_eq!(json["transport"]["kind"], "stdio");
        assert_eq!(
            json["scope"],
            serde_json::json!({ "kind": "workspace", "root": "/Users/me/project" })
        );
        assert_eq!(serde_json::from_value::<McpServer>(json).unwrap(), server);
        assert_eq!(
            server.secret_names().collect::<Vec<_>>(),
            ["GITHUB_PERSONAL_ACCESS_TOKEN"]
        );
    }
}
