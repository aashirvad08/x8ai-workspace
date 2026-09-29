//! Recent workspaces, trust and agent approvals, persisted across "launches"
//! (reloads from disk).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use x8ai_workspace::{Approval, ApprovalStore, MAX_RECENT, RecentWorkspaces, TrustStore};

fn dirs(n: usize) -> (tempfile::TempDir, Vec<PathBuf>) {
    let temp = tempfile::tempdir().unwrap();
    let roots = (0..n)
        .map(|i| {
            let root = temp.path().join(format!("project-{i}"));
            fs::create_dir(&root).unwrap();
            fs::canonicalize(root).unwrap()
        })
        .collect();
    (temp, roots)
}

#[test]
fn remembers_recent_workspaces_most_recent_first() {
    let (temp, roots) = dirs(3);
    let file = temp.path().join("state/recent.json");
    let (mut recent, warning) = RecentWorkspaces::load(file.clone());
    assert!(warning.is_none());
    for root in &roots {
        recent.record(root).unwrap();
    }
    recent.record(&roots[0]).unwrap();

    let (reloaded, _) = RecentWorkspaces::load(file);
    let listed: Vec<_> = reloaded.list().into_iter().map(|w| w.name).collect();
    assert_eq!(listed, ["project-0", "project-2", "project-1"]);
    assert!(reloaded.contains(&roots[1]));
}

#[test]
fn keeps_a_bounded_list() {
    let (temp, roots) = dirs(MAX_RECENT + 3);
    let (mut recent, _) = RecentWorkspaces::load(temp.path().join("recent.json"));
    for root in &roots {
        recent.record(root).unwrap();
    }
    assert_eq!(recent.list().len(), MAX_RECENT);
    assert!(
        !recent.contains(&roots[0]),
        "the oldest entries are dropped"
    );
}

#[test]
fn reports_missing_folders_and_removes_them_on_request() {
    let (temp, roots) = dirs(2);
    let file = temp.path().join("recent.json");
    let (mut recent, _) = RecentWorkspaces::load(file.clone());
    recent.record(&roots[0]).unwrap();
    recent.record(&roots[1]).unwrap();
    fs::remove_dir(&roots[0]).unwrap();

    let listed = recent.list();
    assert!(listed.iter().any(|w| w.name == "project-0" && !w.available));
    assert!(listed.iter().any(|w| w.name == "project-1" && w.available));

    recent.remove(&roots[0]).unwrap();
    let (reloaded, _) = RecentWorkspaces::load(file);
    assert_eq!(reloaded.list().len(), 1);
}

#[test]
fn stores_only_locations_readable_by_the_user_alone() {
    let (temp, roots) = dirs(1);
    fs::write(roots[0].join("secret.env"), "API_KEY=do-not-store").unwrap();
    let file = temp.path().join("state/recent.json");
    let (mut recent, _) = RecentWorkspaces::load(file.clone());
    recent.record(&roots[0]).unwrap();

    let stored = fs::read_to_string(&file).unwrap();
    assert!(!stored.contains("secret") && !stored.contains("API_KEY"));
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
fn moves_a_damaged_file_aside_instead_of_destroying_it() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("recent.json");
    fs::write(&file, "{ not json").unwrap();

    let (recent, warning) = RecentWorkspaces::load(file.clone());
    assert!(recent.list().is_empty());
    assert!(warning.unwrap().contains("could not be read"));
    assert_eq!(
        fs::read_to_string(temp.path().join("recent.json.corrupt")).unwrap(),
        "{ not json"
    );
}

#[test]
fn ignores_relative_or_unexpected_entries() {
    let temp = tempfile::tempdir().unwrap();
    let file = temp.path().join("trust.json");
    fs::write(
        &file,
        r#"{"version":1,"workspaces":[{"root":"relative/path","at":1},{"root":"/","at":2}]}"#,
    )
    .unwrap();
    let (trust, warning) = TrustStore::load(file);
    assert!(warning.is_none());
    assert!(!trust.is_trusted(std::path::Path::new("relative/path")));
    assert!(trust.is_trusted(std::path::Path::new("/")));
}

#[test]
fn trust_is_explicit_exact_and_persistent() {
    let (temp, roots) = dirs(1);
    let file = temp.path().join("trust.json");
    let nested = roots[0].join("sub");
    fs::create_dir(&nested).unwrap();

    let (mut trust, _) = TrustStore::load(file.clone());
    assert!(!trust.is_trusted(&roots[0]), "folders start untrusted");
    trust.set(&roots[0], true).unwrap();

    let (mut reloaded, _) = TrustStore::load(file.clone());
    assert!(reloaded.is_trusted(&roots[0]));
    assert!(
        !reloaded.is_trusted(&nested),
        "trust is not inherited by folders inside"
    );
    assert!(!reloaded.is_trusted(temp.path()), "or by the parent");

    reloaded.set(&roots[0], false).unwrap();
    let (revoked, _) = TrustStore::load(file);
    assert!(!revoked.is_trusted(&roots[0]));
}

fn approval<'a>(root: &'a Path, program: &'a Path, args: &'a [String]) -> Approval<'a> {
    Approval {
        root,
        agent: "claude-code",
        program,
        args,
    }
}

#[test]
fn approvals_survive_a_restart_and_apply_to_one_folder_only() {
    let (temp, roots) = dirs(2);
    let file = temp.path().join("state/approvals.json");
    let program = Path::new("/Users/me/.local/bin/claude");
    let nested = roots[0].join("sub");
    fs::create_dir(&nested).unwrap();

    let (mut approvals, warning) = ApprovalStore::load(file.clone());
    assert!(warning.is_none());
    assert!(!approvals.is_approved(&approval(&roots[0], program, &[])));
    approvals
        .approve(&approval(&roots[0], program, &[]))
        .unwrap();

    let (reloaded, _) = ApprovalStore::load(file.clone());
    assert!(reloaded.is_approved(&approval(&roots[0], program, &[])));
    // Another workspace cannot use it, nor a folder inside or around this one.
    assert!(!reloaded.is_approved(&approval(&roots[1], program, &[])));
    assert!(!reloaded.is_approved(&approval(&nested, program, &[])));
    assert!(!reloaded.is_approved(&approval(temp.path(), program, &[])));
    // Nor another agent.
    let other = Approval {
        agent: "opencode",
        ..approval(&roots[0], program, &[])
    };
    assert!(!reloaded.is_approved(&other));
    assert_eq!(
        fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn an_approval_covers_one_program_and_its_arguments() {
    let (temp, roots) = dirs(1);
    let (mut approvals, _) = ApprovalStore::load(temp.path().join("approvals.json"));
    let program = Path::new("/Users/me/.local/bin/claude");
    approvals
        .approve(&approval(&roots[0], program, &[]))
        .unwrap();

    // A different executable earlier on PATH, or different arguments, is a
    // different launch that the user has not seen.
    let impostor = Path::new("/tmp/evil/claude");
    assert!(!approvals.is_approved(&approval(&roots[0], impostor, &[])));
    let args = ["--dangerously-skip-permissions".to_owned()];
    assert!(!approvals.is_approved(&approval(&roots[0], program, &args)));

    // Approving again replaces the earlier approval for that agent.
    approvals
        .approve(&approval(&roots[0], impostor, &[]))
        .unwrap();
    assert!(!approvals.is_approved(&approval(&roots[0], program, &[])));
}

#[test]
fn approvals_can_be_revoked_per_agent_or_per_folder() {
    let (temp, roots) = dirs(1);
    let file = temp.path().join("approvals.json");
    let (mut approvals, _) = ApprovalStore::load(file.clone());
    let program = Path::new("/usr/local/bin/claude");
    let opencode = Approval {
        agent: "opencode",
        program: Path::new("/usr/local/bin/opencode"),
        ..approval(&roots[0], program, &[])
    };
    approvals
        .approve(&approval(&roots[0], program, &[]))
        .unwrap();
    approvals.approve(&opencode).unwrap();

    approvals.revoke(&roots[0], "claude-code").unwrap();
    let (reloaded, _) = ApprovalStore::load(file.clone());
    assert!(!reloaded.is_approved(&approval(&roots[0], program, &[])));
    assert!(reloaded.is_approved(&opencode));

    let (mut reloaded, _) = ApprovalStore::load(file.clone());
    reloaded.revoke_all(&roots[0]).unwrap();
    let (cleared, _) = ApprovalStore::load(file);
    assert!(!cleared.is_approved(&opencode));
}

#[test]
fn a_project_cannot_approve_agents_for_itself() {
    let (temp, roots) = dirs(1);
    let program = Path::new("/usr/local/bin/claude");
    // A repository ships files claiming approval, in every format it might guess.
    let claim = format!(
        r#"{{"version":1,"workspaces":[{{"root":"{}","agents":[{{"id":"claude-code","program":"{}","args":[],"at":0}}]}}]}}"#,
        roots[0].display(),
        program.display()
    );
    for name in [
        "agent-approvals.json",
        ".x8ai/agent-approvals.json",
        ".x8ai.json",
    ] {
        let path = roots[0].join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, &claim).unwrap();
    }
    // The store lives in the app's data directory, and only reads that file.
    let (approvals, _) = ApprovalStore::load(temp.path().join("app-data/agent-approvals.json"));
    assert!(!approvals.is_approved(&approval(&roots[0], program, &[])));
}

#[test]
fn a_damaged_approval_file_approves_nothing() {
    let (temp, roots) = dirs(1);
    let file = temp.path().join("approvals.json");
    fs::write(
        &file,
        r#"{"version":1,"workspaces":[{"root":"relative","agents":[]}], "#,
    )
    .unwrap();
    let (approvals, warning) = ApprovalStore::load(file.clone());
    assert!(warning.is_some());
    assert!(!approvals.is_approved(&approval(&roots[0], Path::new("/bin/sh"), &[])));
    assert!(file.with_extension("json.corrupt").exists());
}
