//! Recent workspaces and trust, persisted across "launches" (reloads from disk).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use x8ai_workspace::{MAX_RECENT, RecentWorkspaces, TrustStore};

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
