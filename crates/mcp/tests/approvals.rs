//! What must hold before an MCP server runs: trust, an approval of exactly what
//! would run, and an environment with nothing it was not given.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{McpEnvSource, McpEnvVar, McpScope, McpServer, McpServerTransport};
use x8ai_mcp::environment::BASE_VARIABLES;
use x8ai_mcp::{Approvals, Denied, authorize, environment, prepare, secret_account};
use x8ai_secrets::{MemoryStore, SecretStore, SecretValue};
use x8ai_workspace::TrustStore;

const TOKEN: &str = "ghp_x8aitestinvalid000000000000000000000";

struct Machine {
    _temp: tempfile::TempDir,
    root: PathBuf,
    workspace: PathBuf,
    bin: PathBuf,
}

fn machine() -> Machine {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let (workspace, bin) = (root.join("project"), root.join("bin"));
    fs::create_dir_all(&workspace).unwrap();
    fs::create_dir_all(&bin).unwrap();
    for name in ["server", "other-server"] {
        fs::write(bin.join(name), "#!/bin/sh\n").unwrap();
        fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o755)).unwrap();
    }
    Machine {
        _temp: temp,
        root,
        workspace,
        bin,
    }
}

impl Machine {
    fn path(&self) -> String {
        format!("{}:/usr/bin:/bin", self.bin.display())
    }
}

fn server(command: &str, args: &[&str]) -> McpServer {
    McpServer {
        id: IntegrationId::new("github").unwrap(),
        name: "GitHub".into(),
        description: String::new(),
        transport: McpServerTransport::Stdio {
            command: command.into(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
        },
        env: vec![McpEnvVar {
            name: "GITHUB_PERSONAL_ACCESS_TOKEN".into(),
            source: McpEnvSource::Secret,
        }],
        enabled: true,
        scope: McpScope::Global,
    }
}

#[test]
fn a_server_runs_only_in_a_trusted_workspace_once_approved_there() {
    let m = machine();
    let (mut trust, _) = TrustStore::load(m.root.join("data/trusted.json"));
    let (mut approvals, _) = Approvals::load(m.root.join("data/mcp-approvals.json"));
    let prepared = [prepare(&server("server", &["stdio"]), Some(&m.path())).unwrap()];

    assert!(matches!(
        authorize(&m.workspace, &prepared, &trust, &approvals),
        Err(Denied::Untrusted(_))
    ));
    trust.set(&m.workspace, true).unwrap();
    let denied = authorize(&m.workspace, &prepared, &trust, &approvals).unwrap_err();
    assert_eq!(
        denied,
        Denied::NotApproved {
            names: vec!["GitHub".into()],
            changed: false
        }
    );

    approvals
        .approve(&m.workspace, &[("github", &prepared[0].material)])
        .unwrap();
    assert!(authorize(&m.workspace, &prepared, &trust, &approvals).is_ok());
    // Only in that workspace.
    let other = m.root.join("other");
    fs::create_dir_all(&other).unwrap();
    trust.set(&other, true).unwrap();
    assert!(authorize(&other, &prepared, &trust, &approvals).is_err());

    // Approvals survive a restart; removing trust ends them.
    let (reloaded, _) = Approvals::load(m.root.join("data/mcp-approvals.json"));
    assert!(authorize(&m.workspace, &prepared, &trust, &reloaded).is_ok());
    trust.set(&m.workspace, false).unwrap();
    assert!(matches!(
        authorize(&m.workspace, &prepared, &trust, &reloaded),
        Err(Denied::Untrusted(_))
    ));
    let (mut reloaded, _) = Approvals::load(m.root.join("data/mcp-approvals.json"));
    reloaded.revoke_all(&m.workspace).unwrap();
    trust.set(&m.workspace, true).unwrap();
    assert!(authorize(&m.workspace, &prepared, &trust, &reloaded).is_err());
}

#[test]
fn a_changed_command_arguments_endpoint_or_variables_need_a_new_approval() {
    let m = machine();
    let (mut trust, _) = TrustStore::load(m.root.join("trusted.json"));
    let (mut approvals, _) = Approvals::load(m.root.join("mcp-approvals.json"));
    trust.set(&m.workspace, true).unwrap();
    let original = server("server", &["stdio"]);
    let approved = prepare(&original, Some(&m.path())).unwrap();
    approvals
        .approve(&m.workspace, &[("github", &approved.material)])
        .unwrap();

    let check = |changed: McpServer| {
        let prepared = [prepare(&changed, Some(&m.path())).unwrap()];
        authorize(&m.workspace, &prepared, &trust, &approvals).map(|_| ())
    };
    assert!(check(original.clone()).is_ok());

    // Name, description, scope, enabled: not what runs.
    let mut cosmetic = original.clone();
    cosmetic.name = "GitHub (work)".into();
    cosmetic.description = "Issues and PRs".into();
    cosmetic.scope = McpScope::Session;
    assert!(check(cosmetic).is_ok());

    let changed = |transport| McpServer {
        transport,
        ..original.clone()
    };
    let refused = check(changed(McpServerTransport::Stdio {
        command: "other-server".into(),
        args: vec!["stdio".into()],
    }))
    .unwrap_err();
    assert!(
        matches!(refused, Denied::NotApproved { changed: true, .. }),
        "{refused}"
    );
    assert!(refused.to_string().contains("changed since it was allowed"));
    assert!(
        check(changed(McpServerTransport::Stdio {
            command: "server".into(),
            args: vec!["stdio".into(), "--write".into()],
        }))
        .is_err(),
        "an argument"
    );
    let mut variables = original.clone();
    variables.env.push(McpEnvVar {
        name: "AWS_SECRET_ACCESS_KEY".into(),
        source: McpEnvSource::Inherit,
    });
    assert!(check(variables).is_err(), "a variable");
    let mut source = original.clone();
    source.env[0].source = McpEnvSource::Inherit;
    assert!(check(source).is_err(), "a variable's source");

    // The same name found somewhere else on PATH is a different program.
    let earlier = m.root.join("earlier");
    fs::create_dir_all(&earlier).unwrap();
    fs::write(earlier.join("server"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(earlier.join("server"), fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:{}", earlier.display(), m.path());
    let prepared = [prepare(&original, Some(&path)).unwrap()];
    assert!(authorize(&m.workspace, &prepared, &trust, &approvals).is_err());
}

#[test]
fn an_http_endpoint_change_needs_a_new_approval() {
    let m = machine();
    let (mut trust, _) = TrustStore::load(m.root.join("trusted.json"));
    let (mut approvals, _) = Approvals::load(m.root.join("mcp-approvals.json"));
    trust.set(&m.workspace, true).unwrap();
    let remote = |url: &str| McpServer {
        transport: McpServerTransport::StreamableHttp { url: url.into() },
        env: Vec::new(),
        ..server("x", &[])
    };
    let approved = prepare(&remote("https://mcp.example.com/mcp"), None).unwrap();
    approvals
        .approve(&m.workspace, &[("github", &approved.material)])
        .unwrap();
    let prepared = [prepare(&remote("https://mcp.example.com/mcp"), None).unwrap()];
    assert!(authorize(&m.workspace, &prepared, &trust, &approvals).is_ok());
    let prepared = [prepare(&remote("https://mcp.example.org/mcp"), None).unwrap()];
    assert!(authorize(&m.workspace, &prepared, &trust, &approvals).is_err());
}

#[test]
fn a_missing_command_is_reported_and_never_resolved_in_the_workspace() {
    let m = machine();
    let err = prepare(&server("not-installed", &[]), Some(&m.path())).unwrap_err();
    assert!(err.to_string().contains("not found on your PATH"), "{err}");
    // A relative PATH entry would resolve against the directory we happen to be in.
    fs::write(m.workspace.join("server"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(
        m.workspace.join("server"),
        fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    assert!(prepare(&server("server", &[]), Some(".:project")).is_err());
}

#[test]
fn a_servers_environment_is_the_base_its_variables_and_nothing_else() {
    let store = MemoryStore::default();
    store
        .set(
            &secret_account("github", "GITHUB_PERSONAL_ACCESS_TOKEN"),
            &SecretValue::new(TOKEN).unwrap(),
        )
        .unwrap();
    store
        .set(
            &secret_account("other", "OTHER_TOKEN"),
            &SecretValue::new("other-server-secret").unwrap(),
        )
        .unwrap();
    let login: Vec<(String, String)> = [
        ("PATH", "/opt/homebrew/bin:/usr/bin:/bin"),
        ("HOME", "/Users/me"),
        ("USER", "me"),
        ("LANG", "en_GB.UTF-8"),
        ("LC_CTYPE", "UTF-8"),
        ("HTTPS_PROXY", "http://proxy.example:3128"),
        // What the login shell may carry, and a server must not see:
        ("ANTHROPIC_API_KEY", "sk-ant-shell-invalid"),
        ("OPENAI_API_KEY", "sk-openai-shell-invalid"),
        ("OPENROUTER_API_KEY", "sk-or-shell-invalid"),
        ("AWS_SECRET_ACCESS_KEY", "aws-shell-invalid"),
        ("GITHUB_TOKEN", "ghp-shell-invalid"),
        ("EDITOR", "vim"),
    ]
    .into_iter()
    .map(|(n, v)| (n.to_owned(), v.to_owned()))
    .collect();
    let mut github = server("npx", &[]);
    github.env.push(McpEnvVar {
        name: "HTTPS_PROXY".into(),
        source: McpEnvSource::Inherit,
    });
    github.env.push(McpEnvVar {
        name: "NOT_SET_ANYWHERE".into(),
        source: McpEnvSource::Inherit,
    });

    let env = environment(&github, &login, &store).unwrap();
    let mut names = env.names();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "GITHUB_PERSONAL_ACCESS_TOKEN",
            "HOME",
            "HTTPS_PROXY",
            "LANG",
            "LC_CTYPE",
            "PATH",
            "USER"
        ]
    );
    let value = |name: &str| {
        env.vars()
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    };
    assert_eq!(value("GITHUB_PERSONAL_ACCESS_TOKEN"), Some(TOKEN));
    assert_eq!(value("HTTPS_PROXY"), Some("http://proxy.example:3128"));
    for leaked in [
        "ANTHROPIC_API_KEY",
        "OPENAI_API_KEY",
        "OPENROUTER_API_KEY",
        "AWS_SECRET_ACCESS_KEY",
        "GITHUB_TOKEN",
        "EDITOR",
        "OTHER_TOKEN",
    ] {
        assert_eq!(value(leaked), None, "{leaked}");
    }
    assert!(names.iter().all(|n| BASE_VARIABLES.contains(n)
        || n.starts_with("LC_")
        || github.env.iter().any(|v| v.name == *n)));

    // Printing it never shows a value; its values are redacted from text.
    let printed = format!("{env:?}");
    assert!(
        printed.contains("GITHUB_PERSONAL_ACCESS_TOKEN") && !printed.contains(TOKEN),
        "{printed}"
    );
    assert_eq!(
        env.redact(&format!("bad credentials: {TOKEN}")),
        "bad credentials: <redacted>"
    );
}

#[test]
fn a_server_with_a_missing_secret_does_not_start_half_configured() {
    let store = MemoryStore::default();
    let error = environment(&server("npx", &[]), &[], &store).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("GITHUB_PERSONAL_ACCESS_TOKEN is not saved"),
        "{error}"
    );
    assert!(!error.to_string().contains("ghp_"));
}

#[test]
fn secrets_live_in_the_secret_store_only() {
    let m = machine();
    let (mut approvals, _) = Approvals::load(m.root.join("data/mcp-approvals.json"));
    let store = MemoryStore::default();
    let github = server("server", &["stdio"]);
    store
        .set(
            &secret_account("github", "GITHUB_PERSONAL_ACCESS_TOKEN"),
            &SecretValue::new(TOKEN).unwrap(),
        )
        .unwrap();
    let prepared = prepare(&github, Some(&m.path())).unwrap();
    approvals
        .approve(&m.workspace, &[("github", &prepared.material)])
        .unwrap();
    let _ = environment(&github, &[], &store).unwrap();
    let text = fs::read_to_string(m.root.join("data/mcp-approvals.json")).unwrap();
    assert!(!text.contains(TOKEN) && text.contains("GITHUB_PERSONAL_ACCESS_TOKEN"));
    assert!(!format!("{prepared:?}").contains(TOKEN));
    assert!(Path::new(&m.root).join("data").read_dir().unwrap().count() == 1);
}
