//! `x8ai-git` against real repositories made with the real `git`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use x8ai_git::{FileStatus, Git};

/// The test's own git, with an identity for commits made by the tests.
fn sh_git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .current_dir(dir)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_owned()
}

fn git() -> Git {
    Git::new(PathBuf::from("git"), &std::env::vars().collect::<Vec<_>>())
}

/// A repository with one commit: README.md and src/main.rs.
fn repo() -> (tempfile::TempDir, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap().join("project");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("README.md"), "# project\n").unwrap();
    fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    sh_git(&root, &["init", "-q"]);
    sh_git(&root, &["add", "."]);
    sh_git(&root, &["commit", "-qm", "first"]);
    (temp, root)
}

fn worktree_path(temp: &tempfile::TempDir, name: &str) -> PathBuf {
    fs::canonicalize(temp.path())
        .unwrap()
        .join("worktrees")
        .join(name)
}

#[test]
fn describes_a_repository_from_inside_it() {
    let (_t, root) = repo();
    let head = sh_git(&root, &["rev-parse", "HEAD"]);
    let repo = git().repository(&root.join("src")).unwrap().unwrap();
    assert_eq!(repo.toplevel, root);
    assert_eq!(repo.common_dir, root.join(".git"));
    assert_eq!(repo.prefix, "src/");
    assert_eq!(repo.head.as_deref(), Some(head.as_str()));
    assert_eq!(repo.branch.as_deref(), Some("main"));
}

#[test]
fn a_plain_folder_is_not_a_repository_and_a_new_one_has_no_head() {
    let temp = tempfile::tempdir().unwrap();
    let plain = fs::canonicalize(temp.path()).unwrap();
    assert!(git().repository(&plain).unwrap().is_none());

    let fresh = plain.join("fresh");
    fs::create_dir(&fresh).unwrap();
    sh_git(&fresh, &["init", "-q"]);
    let repo = git().repository(&fresh).unwrap().unwrap();
    assert!(repo.head.is_none());
}

#[test]
fn a_worktree_leaves_the_users_working_tree_alone() {
    let (temp, root) = repo();
    let git = git();
    let repo = git.repository(&root).unwrap().unwrap();
    let head = repo.head.clone().unwrap();
    let path = worktree_path(&temp, "claude-code-1");

    git.add_worktree(&repo, &path, "agent/claude-code/1", &head)
        .unwrap();
    assert_eq!(
        fs::read_to_string(path.join("README.md")).unwrap(),
        "# project\n"
    );
    assert!(git.branch_exists(&repo, "agent/claude-code/1").unwrap());
    let listed = git.worktrees(&repo).unwrap();
    assert!(
        listed
            .iter()
            .any(|w| w.path == path && w.branch.as_deref() == Some("agent/claude-code/1"))
    );

    // Work in the worktree does not reach the user's tree or branch.
    fs::write(path.join("README.md"), "# changed by an agent\n").unwrap();
    sh_git(&path, &["commit", "-qam", "agent work"]);
    assert_eq!(
        fs::read_to_string(root.join("README.md")).unwrap(),
        "# project\n"
    );
    assert!(git.is_clean(&root).unwrap());
    assert_eq!(sh_git(&root, &["rev-parse", "HEAD"]), head);
    assert_eq!(sh_git(&root, &["symbolic-ref", "--short", "HEAD"]), "main");
}

#[test]
fn reports_what_changed_in_a_worktree_since_its_base() {
    let (temp, root) = repo();
    let git = git();
    let repo = git.repository(&root).unwrap().unwrap();
    let base = repo.head.clone().unwrap();
    let path = worktree_path(&temp, "opencode-1");
    git.add_worktree(&repo, &path, "agent/opencode/1", &base)
        .unwrap();

    // One committed change, one uncommitted change, one new file, one deletion.
    fs::write(
        path.join("src/main.rs"),
        "fn main() { println!(\"hi\"); }\n",
    )
    .unwrap();
    sh_git(&path, &["commit", "-qam", "say hi"]);
    fs::write(path.join("README.md"), "# project\n\nMore.\n").unwrap();
    fs::write(path.join("notes.txt"), "a new file\n").unwrap();

    let changes = git.changes(&path, &base).unwrap();
    assert_eq!(changes.base, base);
    assert_eq!(changes.commits, 1);
    assert!(changes.uncommitted);
    assert_eq!(changes.branch.as_deref(), Some("agent/opencode/1"));
    let status = |p: &str| changes.files.iter().find(|f| f.path == p).map(|f| f.status);
    assert_eq!(status("src/main.rs"), Some(FileStatus::Modified));
    assert_eq!(status("README.md"), Some(FileStatus::Modified));
    assert_eq!(status("notes.txt"), Some(FileStatus::Untracked));
    assert!(
        changes.diff.contains("+fn main() { println!(\"hi\"); }"),
        "{}",
        changes.diff
    );
    assert!(changes.diff.contains("+More."));
    assert!(changes.diff.contains("+a new file"));
    assert!(!changes.truncated);

    // The user's tree has none of it.
    assert!(git.is_clean(&root).unwrap());
    assert!(!root.join("notes.txt").exists());
}

#[test]
fn removing_a_worktree_with_changes_needs_force() {
    let (temp, root) = repo();
    let git = git();
    let repo = git.repository(&root).unwrap().unwrap();
    let head = repo.head.clone().unwrap();
    let path = worktree_path(&temp, "claude-code-2");
    git.add_worktree(&repo, &path, "agent/claude-code/2", &head)
        .unwrap();
    fs::write(path.join("scratch.txt"), "work in progress\n").unwrap();

    assert!(git.remove_worktree(&repo, &path, false).is_err());
    assert!(path.exists());
    git.remove_worktree(&repo, &path, true).unwrap();
    assert!(!path.exists());
    assert!(git.worktrees(&repo).unwrap().iter().all(|w| w.path != path));

    // The branch outlives the worktree until deleted explicitly.
    assert!(git.branch_exists(&repo, "agent/claude-code/2").unwrap());
    git.delete_branch(&repo, "agent/claude-code/2").unwrap();
    assert!(!git.branch_exists(&repo, "agent/claude-code/2").unwrap());
}

#[test]
fn a_worktree_deleted_by_hand_is_pruned() {
    let (temp, root) = repo();
    let git = git();
    let repo = git.repository(&root).unwrap().unwrap();
    let head = repo.head.clone().unwrap();
    let path = worktree_path(&temp, "claude-code-3");
    git.add_worktree(&repo, &path, "agent/claude-code/3", &head)
        .unwrap();
    fs::remove_dir_all(&path).unwrap();

    git.remove_worktree(&repo, &path, false).unwrap();
    assert!(git.worktrees(&repo).unwrap().iter().all(|w| w.path != path));
}

#[test]
fn the_apps_git_runs_no_repository_hooks_and_ignores_inherited_git_variables() {
    let (temp, root) = repo();
    // A hook that would run on checkout, and variables that would redirect git.
    let marker = fs::canonicalize(temp.path()).unwrap().join("hook-ran");
    let hook = root.join(".git/hooks/post-checkout");
    fs::write(&hook, format!("#!/bin/sh\ntouch '{}'\n", marker.display())).unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    env.push(("GIT_DIR".into(), "/nonexistent".into()));
    env.push(("GIT_WORK_TREE".into(), "/nonexistent".into()));
    let git = Git::new(PathBuf::from("git"), &env);

    let repo = git.repository(&root).unwrap().expect("GIT_DIR was ignored");
    let head = repo.head.clone().unwrap();
    git.add_worktree(
        &repo,
        &worktree_path(&temp, "claude-code-4"),
        "agent/claude-code/4",
        &head,
    )
    .unwrap();
    assert!(!marker.exists(), "the post-checkout hook ran");
}

#[test]
fn refuses_revisions_and_branches_it_did_not_make() {
    let (temp, root) = repo();
    let git = git();
    let repo = git.repository(&root).unwrap().unwrap();
    let path = worktree_path(&temp, "x");
    assert!(
        git.add_worktree(&repo, &path, "agent/claude-code/5", "HEAD")
            .is_err()
    );
    assert!(
        git.add_worktree(&repo, &path, "main", &repo.head.clone().unwrap())
            .is_err()
    );
    assert!(git.changes(&root, "--output=/tmp/pwned").is_err());
    assert!(!path.exists());
}
