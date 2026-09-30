//! Skills: built-in and user skills, persistence, attachment, and how a session
//! keeps exactly the skills it started with.

use std::collections::BTreeSet;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::McpScopeKind;
use x8ai_core::skill::{SkillInput, SkillScope, SkillSource};
use x8ai_skills::{Error, Mismatch, SkillRegistry, attach, builtin, resolve};

fn input(name: &str, scope: McpScopeKind) -> SkillInput {
    SkillInput {
        name: name.into(),
        description: "A test skill".into(),
        instructions: "Answer in short sentences.".into(),
        allowed_tools: vec!["Read".into()],
        scope,
    }
}

fn id(value: &str) -> IntegrationId {
    IntegrationId::new(value).unwrap()
}

#[test]
fn builtin_skills_are_valid_unique_text() {
    let skills = builtin();
    assert!(skills.len() >= 3);
    let ids: BTreeSet<&str> = skills.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids.len(), skills.len());
    for skill in &skills {
        skill
            .validate()
            .unwrap_or_else(|e| panic!("{}: {e}", skill.id));
        assert_eq!(skill.source, SkillSource::Builtin);
        assert_eq!(
            skill.scope,
            SkillScope::Session,
            "{}: chosen at launch, never forced on",
            skill.id
        );
    }
}

#[test]
fn user_skills_persist_with_builtin_ones_and_versions_follow_changes() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("data/skills.json");
    let (mut registry, warnings) = SkillRegistry::load(file.clone());
    assert!(warnings.is_empty());
    assert!(!file.exists(), "loading writes nothing");
    let added = registry
        .add(&input("HEP analysis", McpScopeKind::Session), None)
        .unwrap();
    assert_eq!(
        (added.id.as_str(), added.version, added.source),
        ("hep-analysis", 1, SkillSource::User)
    );
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );

    // A cosmetic change keeps the version; changed instructions bump it.
    let mut edit = input("HEP analysis", McpScopeKind::Session);
    edit.description = "Particle physics".into();
    assert_eq!(
        registry
            .update("hep-analysis", &edit, None)
            .unwrap()
            .version,
        1
    );
    edit.instructions = "Use ROOT conventions.".into();
    assert_eq!(
        registry
            .update("hep-analysis", &edit, None)
            .unwrap()
            .version,
        2
    );

    let (reloaded, warnings) = SkillRegistry::load(file.clone());
    assert!(warnings.is_empty(), "{warnings:?}");
    let skill = reloaded.get("hep-analysis").unwrap();
    assert_eq!(
        (skill.version, skill.instructions.as_str()),
        (2, "Use ROOT conventions.")
    );
    assert_eq!(reloaded.skills().len(), builtin().len() + 1);
    assert_eq!(reloaded.skills()[0].source, SkillSource::Builtin);

    let (mut reloaded, _) = SkillRegistry::load(file.clone());
    reloaded.remove("hep-analysis").unwrap();
    assert!(SkillRegistry::load(file).0.get("hep-analysis").is_none());
}

#[test]
fn builtin_skills_cannot_be_changed_or_removed_and_ids_never_collide() {
    let temp = tempfile::tempdir().unwrap();
    let (mut registry, _) = SkillRegistry::load(temp.path().join("skills.json"));
    assert!(matches!(
        registry.remove("tests-first"),
        Err(Error::Builtin(_))
    ));
    assert!(matches!(
        registry.update("tests-first", &input("Mine", McpScopeKind::Session), None),
        Err(Error::Builtin(_))
    ));
    // A user skill named like a built-in one gets another id.
    let mine = registry
        .add(&input("Tests first", McpScopeKind::Session), None)
        .unwrap();
    assert_eq!(mine.id.as_str(), "tests-first-2");
}

#[test]
fn a_skill_can_never_hold_a_secret_and_the_file_holds_none() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("skills.json");
    let (mut registry, _) = SkillRegistry::load(file.clone());
    let mut secret = input("Deploy", McpScopeKind::Session);
    secret.instructions = format!("Push with token ghp_{} when done.", "c".repeat(36));
    assert!(matches!(
        registry.add(&secret, None),
        Err(Error::Invalid(_))
    ));
    assert!(!file.exists());
    registry
        .add(&input("Short answers", McpScopeKind::Session), None)
        .unwrap();
    let text = fs::read_to_string(&file).unwrap();
    assert!(!text.contains("ghp_"));
}

#[test]
fn a_damaged_or_forged_file_loads_nothing_it_should_not() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("skills.json");
    fs::write(&file, "{ not json").unwrap();
    let (registry, warnings) = SkillRegistry::load(file.clone());
    assert!(warnings[0].contains("damaged"));
    assert_eq!(
        registry.skills().len(),
        builtin().len(),
        "built-in skills still load"
    );
    assert!(temp.path().join("skills.json.corrupt").exists());

    // Claims to be built in; reuses a built-in id; unknown field; valid.
    fs::write(
        &file,
        r#"{"version":1,"skills":[
          {"id":"sneaky","name":"Sneaky","version":1,"instructions":"x","source":"builtin","scope":{"kind":"global"}},
          {"id":"tests-first","name":"Mine","version":1,"instructions":"x","source":"user","scope":{"kind":"session"}},
          {"id":"extra","name":"Extra","version":1,"instructions":"x","source":"user","scope":{"kind":"session"},"run":"rm -rf ~"},
          {"id":"good","name":"Good","version":3,"instructions":"Be brief.","source":"user","scope":{"kind":"session"}}
        ]}"#,
    )
    .unwrap();
    let (registry, warnings) = SkillRegistry::load(file);
    assert_eq!(warnings.len(), 3, "{warnings:?}");
    assert_eq!(registry.get("good").unwrap().version, 3);
    assert!(registry.get("sneaky").is_none() && registry.get("extra").is_none());
    assert_eq!(
        registry.get("tests-first").unwrap().source,
        SkillSource::Builtin
    );
}

#[test]
fn a_session_gets_global_workspace_and_chosen_skills() {
    let temp = tempfile::tempdir().unwrap();
    let (mut registry, _) = SkillRegistry::load(temp.path().join("skills.json"));
    let here = Path::new("/Users/me/project");
    registry
        .add(&input("Always", McpScopeKind::Global), None)
        .unwrap();
    registry
        .add(&input("Here", McpScopeKind::Workspace), Some(here))
        .unwrap();
    registry
        .add(
            &input("Elsewhere", McpScopeKind::Workspace),
            Some(Path::new("/Users/me/other")),
        )
        .unwrap();
    let skills = registry.skills();
    let names = |chosen: &[IntegrationId]| {
        attach(&skills, here, chosen)
            .unwrap()
            .iter()
            .map(|s| s.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&[]), ["Always", "Here"]);
    assert_eq!(
        names(&[id("python-debugging")]),
        ["Python debugging", "Always", "Here"]
    );
    assert!(
        attach(&skills, here, &[id("always")]).is_err(),
        "not chosen per session"
    );
    assert!(attach(&skills, here, &[id("missing")]).is_err());
}

#[test]
fn a_session_runs_only_with_exactly_the_skills_it_recorded() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("skills.json");
    let (mut registry, _) = SkillRegistry::load(file.clone());
    registry
        .add(&input("HEP analysis", McpScopeKind::Session), None)
        .unwrap();
    let skills = registry.skills();
    let recorded: Vec<_> = attach(
        &skills,
        Path::new("/p"),
        &[id("hep-analysis"), id("tests-first")],
    )
    .unwrap()
    .iter()
    .map(|s| s.reference())
    .collect();
    assert_eq!(resolve(&registry.skills(), &recorded).unwrap().len(), 2);

    // Changed: refused, not upgraded.
    let mut edit = input("HEP analysis", McpScopeKind::Session);
    edit.instructions = "Something else entirely.".into();
    registry.update("hep-analysis", &edit, None).unwrap();
    let error = resolve(&registry.skills(), &recorded).unwrap_err();
    assert_eq!(
        error,
        Mismatch::Changed {
            name: "HEP analysis".into(),
            was: 1,
            now: 2
        }
    );
    assert!(error.to_string().contains("start a new session"));

    // Removed: refused, not substituted.
    registry.remove("hep-analysis").unwrap();
    assert_eq!(
        resolve(&registry.skills(), &recorded).unwrap_err(),
        Mismatch::Removed("hep-analysis".into())
    );
}
