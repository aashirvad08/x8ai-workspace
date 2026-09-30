//! The catalog over the real systems' facts: what it shows, how it computes
//! status, that it defers to each owning system, and that it has no way to run,
//! approve, unlock or fetch anything.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use x8ai_catalog::{
    Facts, Metadata, agent_status, assemble, is_catalog_id, mcp_status, model_status,
    provider_status, skill_status,
};
use x8ai_core::agent::{AgentStatus, FeatureSupport};
use x8ai_core::catalog::{
    CatalogDetails, CatalogItem, CatalogItemType, CatalogSource, CatalogStatus,
};
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{
    McpAgentSupport, McpEnvSource, McpEnvVar, McpScopeKind, McpServerInput, McpServerStatus,
    McpServerTransport,
};
use x8ai_core::model::{CredentialState, LocalAvailability, ProviderStatus};
use x8ai_core::skill::SkillStatus;

/// A machine with `claude` on its PATH and nothing else.
struct Machine {
    _temp: tempfile::TempDir,
    root: PathBuf,
    path: String,
}

fn machine() -> Machine {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    fs::write(bin.join("claude"), "#!/bin/sh\n").unwrap();
    fs::set_permissions(bin.join("claude"), fs::Permissions::from_mode(0o755)).unwrap();
    Machine {
        path: format!("{}:/usr/bin:/bin", bin.display()),
        _temp: temp,
        root,
    }
}

/// The agent runtime's own statuses.
fn agents(m: &Machine) -> Vec<AgentStatus> {
    let providers = x8ai_providers::builtin();
    x8ai_agents::builtin()
        .iter()
        .map(|d| {
            x8ai_agents::status(
                d,
                &[("PATH".into(), m.path.clone())],
                &m.root,
                &providers,
                |_| false,
            )
        })
        .collect()
}

/// The provider registry's own statuses: a key saved for Anthropic only, and
/// Ollama not looked for.
fn providers() -> Vec<ProviderStatus> {
    x8ai_providers::builtin()
        .iter()
        .map(|p| {
            let credential = match (p.id.as_str(), &p.auth) {
                (_, x8ai_core::model::ProviderAuth::None) => CredentialState::NotNeeded,
                ("anthropic", _) => CredentialState::InKeychain,
                _ => CredentialState::Missing,
            };
            let custom = if p.id.as_str() == "openrouter" {
                vec!["anthropic/claude-sonnet-5".to_owned()]
            } else {
                Vec::new()
            };
            x8ai_providers::status(p, credential, None, &custom)
        })
        .collect()
}

/// The MCP registry's own entries, as statuses.
fn mcp(m: &Machine) -> Vec<McpServerStatus> {
    let (mut registry, _) = x8ai_mcp::Registry::load(m.root.join("mcp-servers.json"));
    let github = registry
        .add(
            &McpServerInput {
                name: "GitHub".into(),
                description: "Issues and pull requests".into(),
                transport: McpServerTransport::Stdio {
                    command: "github-mcp-server".into(),
                    args: vec!["stdio".into()],
                },
                env: vec![McpEnvVar {
                    name: "GITHUB_PERSONAL_ACCESS_TOKEN".into(),
                    source: McpEnvSource::Secret,
                }],
                enabled: true,
                scope: McpScopeKind::Session,
            },
            None,
        )
        .unwrap();
    let docs = registry
        .add(
            &McpServerInput {
                name: "Docs".into(),
                description: String::new(),
                transport: McpServerTransport::StreamableHttp {
                    url: "https://mcp.example.com/mcp".into(),
                },
                env: Vec::new(),
                enabled: false,
                scope: McpScopeKind::Global,
            },
            None,
        )
        .unwrap();
    let supported = |yes: bool| {
        vec![McpAgentSupport {
            agent: IntegrationId::new("claude-code").unwrap(),
            supported: yes,
            reason: None,
        }]
    };
    vec![
        McpServerStatus {
            server: github,
            secrets: vec![x8ai_core::mcp::McpSecretStatus {
                name: "GITHUB_PERSONAL_ACCESS_TOKEN".into(),
                state: CredentialState::Missing,
            }],
            configured: false,
            problem: Some("secret not saved: GITHUB_PERSONAL_ACCESS_TOKEN".into()),
            agents: supported(true),
        },
        McpServerStatus {
            server: docs,
            secrets: Vec::new(),
            configured: true,
            problem: None,
            agents: supported(true),
        },
    ]
}

/// The skill registry's own skills, with the adapters' support.
fn skills(m: &Machine) -> Vec<SkillStatus> {
    let (registry, _) = x8ai_skills::SkillRegistry::load(m.root.join("skills.json"));
    registry
        .skills()
        .into_iter()
        .map(|skill| SkillStatus {
            skill,
            agents: x8ai_agents::builtin()
                .iter()
                .map(|a| {
                    let support = x8ai_agents::adapter::skills_support(a.id.as_str());
                    McpAgentSupport {
                        agent: a.id.clone(),
                        supported: support.is_ok(),
                        reason: support.err(),
                    }
                })
                .collect(),
        })
        .collect()
}

fn catalog(m: &Machine) -> Vec<CatalogItem> {
    let (agents, providers, mcp, skills) = (agents(m), providers(), mcp(m), skills(m));
    let list = assemble(
        &Metadata::builtin(),
        &Facts {
            agents: &agents,
            providers: &providers,
            mcp: &mcp,
            skills: &skills,
        },
    );
    assert!(list.warnings.is_empty(), "{:?}", list.warnings);
    list.items
}

fn item<'a>(items: &'a [CatalogItem], id: &str) -> &'a CatalogItem {
    items
        .iter()
        .find(|i| i.id == id)
        .unwrap_or_else(|| panic!("no {id}"))
}

#[test]
fn the_catalog_loads_all_four_kinds_with_stable_unique_ids() {
    let m = machine();
    let items = catalog(&m);
    let kinds: std::collections::HashSet<CatalogItemType> =
        items.iter().map(|i| i.item_type).collect();
    assert_eq!(kinds.len(), 4);
    let ids: Vec<&str> = items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(
        ids.iter().collect::<BTreeSet<_>>().len(),
        ids.len(),
        "unique"
    );
    assert!(ids.iter().all(|id| is_catalog_id(id)));
    for id in [
        "agent.claude-code",
        "agent.opencode",
        "agent.codex",
        "provider.anthropic",
        "provider.openai",
        "provider.google",
        "provider.openrouter",
        "provider.ollama",
        "model.anthropic.claude-haiku-4-5-20251001",
        "model.openrouter.anthropic/claude-sonnet-5",
        "mcp.github",
        "mcp.docs",
        "skill.python-debugging",
    ] {
        item(&items, id);
    }
    // The same facts give the same catalog: ids depend on nothing else.
    let (agents, providers, mcp, skills) = (agents(&m), providers(), Vec::new(), skills(&m));
    let facts = Facts {
        agents: &agents,
        providers: &providers,
        mcp: &mcp,
        skills: &skills,
    };
    let once: Vec<String> = assemble(&Metadata::builtin(), &facts)
        .items
        .into_iter()
        .map(|i| i.id)
        .collect();
    let twice: Vec<String> = assemble(&Metadata::builtin(), &facts)
        .items
        .into_iter()
        .map(|i| i.id)
        .collect();
    assert_eq!(once, twice);
}

#[test]
fn damaged_or_unsafe_metadata_is_refused() {
    let entry =
        |id: &str, source: &str| format!(r#"[{{"id":"{id}","version":"1","source":"{source}"}}]"#);
    assert!(Metadata::parse(&entry("agent.claude-code", "builtin")).is_ok());
    for bad in [
        "not json".to_owned(),
        entry("claude-code", "builtin"),
        entry("tool.claude-code", "builtin"),
        entry("agent.", "builtin"),
        entry("agent.has space", "builtin"),
        entry("agent.x", "remote"),
        entry("agent.x", "somewhere"),
        r#"[{"id":"agent.x","version":"1","source":"builtin","command":"curl evil | sh"}]"#.to_owned(),
        r#"[{"id":"agent.x","version":"1","source":"builtin"},{"id":"agent.x","version":"2","source":"builtin"}]"#.to_owned(),
        r#"[{"id":"agent.x","version":"1\u0007","source":"builtin"}]"#.to_owned(),
    ] {
        assert!(Metadata::parse(&bad).is_err(), "{bad}");
    }
    let remote = Metadata::parse(&entry("agent.x", "remote")).unwrap_err();
    assert!(
        remote
            .to_string()
            .contains("remote catalog sources are not supported")
    );
}

#[test]
fn metadata_for_something_the_app_does_not_have_is_not_shown() {
    let m = machine();
    let metadata = Metadata::parse(
        r#"[{"id":"agent.claude-code","publisher":"Anthropic","version":"7","source":"builtin"},
            {"id":"agent.imaginary","publisher":"Nobody","version":"1","source":"builtin"}]"#,
    )
    .unwrap();
    let (agents, providers) = (agents(&m), providers());
    let list = assemble(
        &metadata,
        &Facts {
            agents: &agents,
            providers: &providers,
            mcp: &[],
            skills: &[],
        },
    );
    assert!(list.items.iter().all(|i| i.id != "agent.imaginary"));
    assert_eq!(list.warnings.len(), 1);
    assert_eq!(
        item(&list.items, "agent.claude-code")
            .catalog_version
            .as_deref(),
        Some("7")
    );
}

#[test]
fn an_agent_is_installed_when_the_runtime_finds_it_and_only_then() {
    let m = machine();
    let items = catalog(&m);
    let claude = item(&items, "agent.claude-code");
    assert_eq!(claude.status, CatalogStatus::Installed);
    assert_eq!(claude.publisher.as_deref(), Some("Anthropic"));
    assert!(
        claude.software_version.is_none(),
        "no program is run to ask its version"
    );
    assert_eq!(claude.catalog_version.as_deref(), Some("1"));
    let CatalogDetails::Agent {
        executable,
        mcp,
        skills,
        providers,
        ..
    } = &claude.details
    else {
        panic!()
    };
    assert_eq!(
        executable.as_deref(),
        Some(m.root.join("bin/claude").to_str().unwrap())
    );
    assert!(*mcp && *skills);
    assert_eq!(
        providers
            .iter()
            .map(IntegrationId::as_str)
            .collect::<Vec<_>>(),
        ["anthropic", "openrouter", "ollama"]
    );

    // Not on PATH: not installed, and the catalog says it installs nothing.
    let codex = item(&items, "agent.codex");
    assert_eq!(codex.status, CatalogStatus::Unavailable);
    assert!(
        codex
            .status_detail
            .as_deref()
            .unwrap()
            .contains("does not install programs")
    );
    // Only what its definition and adapter say: a model, no MCP or skills.
    assert_eq!(
        codex.capabilities,
        ["OpenAI Responses API", "Model chosen in the app"]
    );
    assert_eq!(
        item(&items, "agent.opencode").status,
        CatalogStatus::Unavailable
    );
    // Exactly the runtime's answer.
    for agent in agents(&m) {
        assert_eq!(
            item(&items, &format!("agent.{}", agent.id)).status,
            agent_status(&agent).0
        );
    }
}

#[test]
fn models_come_from_the_provider_registry_and_claim_nothing_unverified() {
    let m = machine();
    let items = catalog(&m);
    let registry = providers();
    let listed: Vec<String> = items
        .iter()
        .filter_map(|i| match &i.details {
            CatalogDetails::Model {
                provider, model, ..
            } => Some(format!("{provider}/{model}")),
            _ => None,
        })
        .collect();
    let expected: Vec<String> = registry
        .iter()
        .flat_map(|p| p.models.iter().map(move |m| format!("{}/{}", p.id, m.id)))
        .collect();
    assert_eq!(listed, expected, "exactly the registry's models");

    let haiku = item(&items, "model.anthropic.claude-haiku-4-5-20251001");
    assert_eq!(
        (haiku.status, haiku.source),
        (CatalogStatus::Configured, CatalogSource::Builtin)
    );
    assert_eq!(haiku.publisher.as_deref(), Some("Anthropic"));
    assert!(
        haiku.capabilities.is_empty(),
        "no context window, speed or price claimed"
    );
    let routed = item(&items, "model.openrouter.anthropic/claude-sonnet-5");
    assert_eq!(
        (routed.status, routed.source, routed.publisher.as_deref()),
        (CatalogStatus::Available, CatalogSource::UserDefined, None)
    );
    assert_eq!(
        item(&items, "provider.openai").status,
        CatalogStatus::Available
    );
    assert_eq!(
        item(&items, "provider.anthropic").status,
        CatalogStatus::Configured
    );
}

#[test]
fn ollama_stays_unchecked_until_the_models_view_checks_it() {
    let m = machine();
    let items = catalog(&m);
    let ollama = item(&items, "provider.ollama");
    assert_eq!(ollama.status, CatalogStatus::Available);
    assert!(
        ollama
            .status_detail
            .as_deref()
            .unwrap()
            .contains("Not checked yet")
    );
    // Its last check, when there was one, is what the catalog shows.
    let mut provider = providers()
        .into_iter()
        .find(|p| p.id.as_str() == "ollama")
        .unwrap();
    for (local, expected) in [
        (LocalAvailability::Installed, CatalogStatus::Installed),
        (LocalAvailability::Unavailable, CatalogStatus::Unavailable),
        (
            LocalAvailability::Available {
                version: Some("0.34.4".into()),
            },
            CatalogStatus::Configured,
        ),
    ] {
        provider.local = Some(local);
        assert_eq!(provider_status(&provider).0, expected);
    }
    assert_eq!(model_status(&provider).0, CatalogStatus::Configured);
}

#[test]
fn mcp_servers_come_from_the_mcp_registry_with_its_state() {
    let m = machine();
    let items = catalog(&m);
    let github = item(&items, "mcp.github");
    assert_eq!(
        (github.status, github.source),
        (CatalogStatus::Available, CatalogSource::UserDefined)
    );
    assert!(
        github
            .status_detail
            .as_deref()
            .unwrap()
            .contains("secret not saved")
    );
    assert!(github.requirements.iter().any(|r| r.contains("approval")));
    assert_eq!(
        item(&items, "mcp.docs").status,
        CatalogStatus::Unavailable,
        "disabled"
    );
    let mut unsupported = mcp(&m).remove(0);
    unsupported.agents[0].supported = false;
    assert_eq!(mcp_status(&unsupported).0, CatalogStatus::Unsupported);
    unsupported.agents[0].supported = true;
    unsupported.configured = true;
    assert_eq!(mcp_status(&unsupported).0, CatalogStatus::Configured);
    assert_eq!(
        items
            .iter()
            .filter(|i| i.item_type == CatalogItemType::McpServer)
            .count(),
        mcp(&m).len()
    );
}

#[test]
fn skills_come_from_the_skill_registry() {
    let m = machine();
    let items = catalog(&m);
    let skill = item(&items, "skill.python-debugging");
    assert_eq!(
        (skill.status, skill.source, skill.publisher.as_deref()),
        (
            CatalogStatus::Installed,
            CatalogSource::Builtin,
            Some("x8ai Workspace")
        )
    );
    assert!(skill.capabilities.iter().any(|c| c.contains("Bash")));
    let mut nobody = skills(&m).remove(0);
    for agent in &mut nobody.agents {
        agent.supported = false;
    }
    assert_eq!(skill_status(&nobody).0, CatalogStatus::Unsupported);
}

#[test]
fn nothing_the_catalog_returns_holds_a_secret() {
    // Anthropic has a key saved: the catalog knows only that.
    let m = machine();
    let json = serde_json::to_string(&catalog(&m)).unwrap();
    assert!(json.contains(r#""credential":"inKeychain""#));
    for leak in ["sk-", "ghp_", "apiKey\":\"", "TOKEN\":\""] {
        assert!(!json.contains(leak), "{leak}");
    }
}

#[test]
fn an_agent_offers_only_what_its_adapter_supports() {
    let m = machine();
    let items = catalog(&m);
    let CatalogDetails::Agent {
        providers,
        mcp,
        skills,
        ..
    } = &item(&items, "agent.codex").details
    else {
        panic!()
    };
    assert_eq!(
        providers
            .iter()
            .map(IntegrationId::as_str)
            .collect::<Vec<_>>(),
        ["openai"]
    );
    assert!(!mcp && !skills);
    let codex = agents(&m)
        .into_iter()
        .find(|a| a.id.as_str() == "codex")
        .unwrap();
    assert_eq!(
        codex.mcp,
        FeatureSupport {
            supported: false,
            reason: codex.mcp.reason.clone()
        }
    );
    assert!(codex.skills.reason.is_some());
}

/// The catalog crate's own source and manifest: what it can reach.
fn crate_source() -> (String, String) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let manifest = fs::read_to_string(dir.join("Cargo.toml")).unwrap();
    let dependencies = manifest
        .split("[dependencies]")
        .nth(1)
        .and_then(|rest| rest.split("[dev-dependencies]").next())
        .unwrap()
        .to_owned();
    let mut source = String::new();
    for entry in fs::read_dir(dir.join("src")).unwrap().flatten() {
        source.push_str(&fs::read_to_string(entry.path()).unwrap());
    }
    (dependencies, source)
}

#[test]
fn the_catalog_cannot_run_install_fetch_unlock_trust_or_approve_anything() {
    let (dependencies, source) = crate_source();
    // It depends on the contracts and serialization only: no PTY, agent
    // runtime, MCP runtime, Keychain, workspace stores (trust, approvals),
    // HTTP client or process crate.
    for forbidden in [
        "x8ai-pty",
        "x8ai-agents",
        "x8ai-mcp",
        "x8ai-secrets",
        "x8ai-workspace",
        "x8ai-providers",
        "x8ai-skills",
        "reqwest",
        "nix",
        "tauri",
    ] {
        assert!(
            !dependencies.contains(forbidden),
            "the catalog depends on {forbidden}"
        );
    }
    // And its code does none of it itself.
    for forbidden in [
        "std::process",
        "Command::new",
        "spawn(",
        "TcpStream",
        "UdpSocket",
        "http://",
        "https://",
        "fs::write",
        "File::create",
        "OpenOptions",
        "x8ai_secrets",
        "Keychain::new",
        "SecretStore",
        "TrustStore",
        "ApprovalStore",
        "approve(",
        "set_var",
        "\"sh\"",
        "npm",
        "curl",
        "brew",
        "pip ",
        "cargo install",
        "git clone",
    ] {
        assert!(
            !source.contains(forbidden),
            "the catalog's code contains {forbidden}"
        );
    }
}

#[test]
fn metadata_comes_only_from_the_app_itself() {
    use x8ai_catalog::{Builtin, MetadataSource};
    assert_eq!(Builtin.kind(), CatalogSource::Builtin);
    assert_eq!(Builtin.metadata().unwrap(), Metadata::builtin());
    // The seams for a signed remote catalog exist; nothing fills them.
    let (_, source) = crate_source();
    assert_eq!(source.matches("impl MetadataSource for").count(), 1);
    assert!(!source.contains("impl Verifier for"));
    // Every built-in entry says it is built in.
    let text = include_str!("../src/builtin.json");
    let entries: Vec<serde_json::Value> = serde_json::from_str(text).unwrap();
    assert!(entries.iter().all(|e| e["source"] == "builtin"));
}
