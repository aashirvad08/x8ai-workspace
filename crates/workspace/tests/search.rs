//! Workspace-wide text search against real files.

use std::fs;
use std::os::unix::fs::symlink;
use std::sync::atomic::AtomicBool;

use x8ai_core::workspace::{SearchMatch, SearchQuery};
use x8ai_workspace::{SearchLimits, Workspace};

struct Found {
    files: Vec<(String, Vec<SearchMatch>)>,
    truncated: bool,
}

fn search(workspace: &Workspace, text: &str, case_sensitive: bool, limits: SearchLimits) -> Found {
    let mut files = Vec::new();
    let summary = workspace
        .search(
            &SearchQuery {
                text: text.into(),
                case_sensitive,
            },
            limits,
            &AtomicBool::new(false),
            |path, matches| files.push((path, matches)),
        )
        .unwrap();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(summary.files as usize, files.len());
    assert_eq!(
        summary.matches as usize,
        files.iter().map(|f| f.1.len()).sum::<usize>()
    );
    Found {
        files,
        truncated: summary.truncated,
    }
}

fn paths(found: &Found) -> Vec<&str> {
    found.files.iter().map(|f| f.0.as_str()).collect()
}

fn project() -> (tempfile::TempDir, std::path::PathBuf, Workspace) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("src/main.py"),
        "import os\n\ndef Needle():\n    return 'needle'  # needle\n",
    )
    .unwrap();
    fs::write(root.join("README.md"), "No match here.\n").unwrap();
    let workspace = Workspace::open(&root).unwrap();
    (temp, fs::canonicalize(root).unwrap(), workspace)
}

#[test]
fn finds_lines_and_columns() {
    let (_t, _root, workspace) = project();
    let found = search(&workspace, "needle", false, SearchLimits::default());
    assert_eq!(paths(&found), ["src/main.py"]);
    let matches = &found.files[0].1;
    assert_eq!(matches.len(), 2, "one entry per matching line");
    assert_eq!(
        (matches[0].line, matches[0].column, matches[0].length),
        (3, 4, 6)
    );
    assert_eq!(matches[1].line, 4);
    assert_eq!(
        matches[1].ranges.len(),
        2,
        "every match on the line is marked"
    );
}

#[test]
fn respects_case_sensitivity() {
    let (_t, _root, workspace) = project();
    let found = search(&workspace, "Needle", true, SearchLimits::default());
    assert_eq!(found.files[0].1.len(), 1);
    assert_eq!(found.files[0].1[0].line, 3);
}

#[test]
fn matches_text_literally() {
    let (_t, root, workspace) = project();
    fs::write(root.join("regex.txt"), "a.b\naxb\n(x)\n").unwrap();
    let found = search(&workspace, "a.b", true, SearchLimits::default());
    assert_eq!(found.files.len(), 1);
    assert_eq!(found.files[0].1.len(), 1, "'.' is not a wildcard");
    assert_eq!(
        search(&workspace, "(x)", true, SearchLimits::default())
            .files
            .len(),
        1
    );
}

#[test]
fn skips_generated_ignored_and_binary_files() {
    let (_t, root, workspace) = project();
    for dir in [
        "node_modules/pkg",
        ".git/objects",
        "target/debug",
        "build",
        "ignored",
    ] {
        fs::create_dir_all(root.join(dir)).unwrap();
        fs::write(root.join(dir).join("hit.txt"), "needle\n").unwrap();
    }
    fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    fs::write(root.join("image.bin"), b"needle\0\x01\x02needle").unwrap();
    fs::write(root.join(".env"), "TOKEN=needle\n").unwrap();

    let found = search(&workspace, "needle", false, SearchLimits::default());
    assert_eq!(paths(&found), [".env", "src/main.py"]);
}

#[test]
fn never_leaves_the_workspace_through_symlinks() {
    let (temp, root, workspace) = project();
    let outside = temp.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("secret.txt"), "needle outside\n").unwrap();
    symlink(&outside, root.join("linked-dir")).unwrap();
    symlink(outside.join("secret.txt"), root.join("linked-file.txt")).unwrap();

    let found = search(&workspace, "outside", false, SearchLimits::default());
    assert!(found.files.is_empty(), "found {:?}", paths(&found));
}

#[test]
fn stops_at_its_limits() {
    let (_t, root, workspace) = project();
    fs::write(root.join("many.txt"), "needle\n".repeat(50)).unwrap();
    let limits = SearchLimits {
        max_matches: 1000,
        max_matches_per_file: 10,
        max_file_bytes: 1 << 20,
    };
    let found = search(&workspace, "needle", false, limits);
    let many = found.files.iter().find(|f| f.0 == "many.txt").unwrap();
    assert_eq!(many.1.len(), 10);
    assert!(found.truncated);

    let total = SearchLimits {
        max_matches: 5,
        max_matches_per_file: 100,
        max_file_bytes: 1 << 20,
    };
    let found = search(&workspace, "needle", false, total);
    assert!(found.files.iter().map(|f| f.1.len()).sum::<usize>() <= 5);
    assert!(found.truncated);
}

#[test]
fn can_be_cancelled() {
    let (_t, _root, workspace) = project();
    let summary = workspace
        .search(
            &SearchQuery {
                text: "needle".into(),
                case_sensitive: false,
            },
            SearchLimits::default(),
            &AtomicBool::new(true),
            |_, _| panic!("a cancelled search reports nothing"),
        )
        .unwrap();
    assert!(summary.cancelled);
}

#[test]
fn empty_queries_find_nothing() {
    let (_t, _root, workspace) = project();
    assert!(
        search(&workspace, "", false, SearchLimits::default())
            .files
            .is_empty()
    );
}
