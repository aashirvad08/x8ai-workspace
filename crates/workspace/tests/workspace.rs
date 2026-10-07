//! Workspace operations against real temporary directories.

use std::fs;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

use x8ai_core::workspace::{EntryKind, FileVersion, WorkspaceEvent};
use x8ai_workspace::{ConflictReason, Error, Removal, Workspace};

/// A workspace in a fresh temporary directory, and a directory next to it that
/// must stay unreachable.
struct Fixture {
    _temp: tempfile::TempDir,
    root: std::path::PathBuf,
    outside: std::path::PathBuf,
    workspace: Workspace,
}

fn fixture() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("project");
    let outside = temp.path().join("outside");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    fs::write(root.join("src/main.py"), "print('hi')\n").unwrap();
    fs::write(root.join("README.md"), "# project\n").unwrap();
    fs::write(outside.join("secret.txt"), "top secret").unwrap();
    let workspace = Workspace::open(&root).unwrap();
    Fixture {
        root: fs::canonicalize(&root).unwrap(),
        outside,
        workspace,
        _temp: temp,
    }
}

fn read(root: &Path, rel: &str) -> String {
    fs::read_to_string(root.join(rel)).unwrap()
}

#[test]
fn opens_a_directory_as_a_workspace() {
    let f = fixture();
    let info = f.workspace.info("ws-abcdef", false);
    assert_eq!(info.name, "project");
    assert_eq!(Path::new(&info.root), f.root.as_path());
    assert_eq!(f.workspace.root(), f.root.as_path());
}

#[test]
fn refuses_to_open_files_and_missing_paths() {
    let f = fixture();
    assert!(matches!(
        Workspace::open(&f.root.join("README.md")),
        Err(Error::InvalidPath { .. })
    ));
    assert!(matches!(
        Workspace::open(&f.root.join("nope")),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn lists_one_level_directories_first() {
    let f = fixture();
    fs::create_dir(f.root.join("tests")).unwrap();
    fs::write(f.root.join(".env"), "").unwrap();

    let names: Vec<_> = f
        .workspace
        .list_dir("")
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["src", "tests", ".env", "README.md"]);

    let nested = f.workspace.list_dir("src").unwrap();
    assert_eq!(nested.len(), 1);
    assert_eq!(nested[0].path, "src/main.py");
    assert_eq!(nested[0].kind, EntryKind::File);
    assert!(!nested[0].symlink);
}

#[test]
fn reads_and_saves_text_with_versions() {
    let f = fixture();
    let opened = f.workspace.read_text("src/main.py").unwrap();
    assert_eq!(opened.text, "print('hi')\n");

    let saved = f
        .workspace
        .write_text(
            "src/main.py",
            "print('héllo, 世界 ✓')\n",
            Some(&opened.version),
        )
        .unwrap();
    assert_ne!(saved, opened.version);
    assert_eq!(read(&f.root, "src/main.py"), "print('héllo, 世界 ✓')\n");
    assert_eq!(
        f.workspace.file_version("src/main.py").unwrap(),
        Some(saved)
    );
}

#[test]
fn never_overwrites_an_external_change() {
    let f = fixture();
    let opened = f.workspace.read_text("README.md").unwrap();
    std::thread::sleep(Duration::from_millis(10));
    fs::write(f.root.join("README.md"), "# changed elsewhere, longer\n").unwrap();

    let err = f
        .workspace
        .write_text("README.md", "mine", Some(&opened.version))
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Conflict {
            reason: ConflictReason::Modified,
            ..
        }
    ));
    assert_eq!(read(&f.root, "README.md"), "# changed elsewhere, longer\n");

    // An explicit overwrite goes ahead.
    f.workspace.write_text("README.md", "mine", None).unwrap();
    assert_eq!(read(&f.root, "README.md"), "mine");
}

#[test]
fn reports_a_file_deleted_since_it_was_opened() {
    let f = fixture();
    let opened = f.workspace.read_text("README.md").unwrap();
    fs::remove_file(f.root.join("README.md")).unwrap();
    let err = f
        .workspace
        .write_text("README.md", "mine", Some(&opened.version))
        .unwrap_err();
    assert!(matches!(
        err,
        Error::Conflict {
            reason: ConflictReason::Deleted,
            ..
        }
    ));
    assert_eq!(f.workspace.file_version("README.md").unwrap(), None);
}

#[test]
fn saving_keeps_permissions_and_leaves_no_temporary_files() {
    let f = fixture();
    let script = f.root.join("run.sh");
    fs::write(&script, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

    let opened = f.workspace.read_text("run.sh").unwrap();
    f.workspace
        .write_text("run.sh", "#!/bin/sh\necho hi\n", Some(&opened.version))
        .unwrap();
    assert_eq!(
        fs::metadata(&script).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let names: Vec<_> = f
        .workspace
        .list_dir("")
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(names.iter().all(|n| !n.contains("x8ai-save")), "{names:?}");
}

#[test]
fn saving_through_a_symlink_keeps_the_link() {
    let f = fixture();
    symlink("README.md", f.root.join("link.md")).unwrap();
    let opened = f.workspace.read_text("link.md").unwrap();
    f.workspace
        .write_text("link.md", "via link", Some(&opened.version))
        .unwrap();
    assert!(
        fs::symlink_metadata(f.root.join("link.md"))
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert_eq!(read(&f.root, "README.md"), "via link");
}

#[test]
fn refuses_binary_files() {
    let f = fixture();
    fs::write(
        f.root.join("image.png"),
        [0x89, b'P', b'N', b'G', 0xff, 0xfe],
    )
    .unwrap();
    assert!(matches!(
        f.workspace.read_text("image.png"),
        Err(Error::NotText(_))
    ));
    // Valid UTF-8, but a NUL byte makes it binary, as for search and git.
    fs::write(f.root.join("data.bin"), b"header\0payload").unwrap();
    assert!(matches!(
        f.workspace.read_text("data.bin"),
        Err(Error::NotText(_))
    ));
}

#[test]
fn creates_files_and_directories() {
    let f = fixture();
    f.workspace.create_dir("src/models").unwrap();
    f.workspace.create_file("src/models/train.py").unwrap();
    assert!(f.root.join("src/models").is_dir());
    assert_eq!(read(&f.root, "src/models/train.py"), "");

    assert!(matches!(
        f.workspace.create_file("README.md"),
        Err(Error::AlreadyExists(_))
    ));
    assert!(matches!(
        f.workspace.create_dir("src"),
        Err(Error::AlreadyExists(_))
    ));
    assert!(matches!(
        f.workspace.create_file("missing/x.txt"),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn renames_and_moves_without_replacing() {
    let f = fixture();
    f.workspace.rename("src/main.py", "src/app.py").unwrap();
    assert!(f.root.join("src/app.py").is_file());
    f.workspace.rename("src", "lib").unwrap();
    assert!(f.root.join("lib/app.py").is_file());

    assert!(matches!(
        f.workspace.rename("lib/app.py", "README.md"),
        Err(Error::AlreadyExists(_))
    ));
    assert_eq!(read(&f.root, "README.md"), "# project\n");
    assert!(matches!(
        f.workspace.rename("nope", "x"),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn renames_that_only_change_case() {
    let f = fixture();
    f.workspace.rename("README.md", "Readme.md").unwrap();
    let names: Vec<_> = f
        .workspace
        .list_dir("")
        .unwrap()
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert!(names.contains(&"Readme.md".to_owned()), "{names:?}");
}

#[test]
fn deletes_files_and_directories() {
    let f = fixture();
    f.workspace
        .delete("README.md", Removal::Permanently)
        .unwrap();
    f.workspace.delete("src", Removal::Permanently).unwrap();
    assert!(!f.root.join("README.md").exists());
    assert!(!f.root.join("src").exists());
    assert!(matches!(
        f.workspace.delete("src", Removal::Permanently),
        Err(Error::NotFound(_))
    ));
}

#[test]
fn deleting_a_symlink_keeps_its_target() {
    let f = fixture();
    symlink(&f.outside, f.root.join("out")).unwrap();
    f.workspace.delete("out", Removal::Permanently).unwrap();
    assert!(f.outside.join("secret.txt").exists());
}

#[test]
fn rejects_paths_that_leave_the_workspace() {
    let f = fixture();
    for bad in [
        "../outside/secret.txt",
        "/etc/passwd",
        "src/../../outside/secret.txt",
    ] {
        assert!(
            matches!(f.workspace.read_text(bad), Err(Error::InvalidPath { .. })),
            "{bad}"
        );
        assert!(f.workspace.write_text(bad, "x", None).is_err(), "{bad}");
        assert!(
            f.workspace.delete(bad, Removal::Permanently).is_err(),
            "{bad}"
        );
    }
    assert!(matches!(
        f.workspace.rename("README.md", "../escaped.md"),
        Err(Error::InvalidPath { .. })
    ));
    assert!(f.outside.join("secret.txt").exists());
}

#[test]
fn symlinks_cannot_reach_outside() {
    let f = fixture();
    symlink(&f.outside, f.root.join("out")).unwrap();
    symlink(f.outside.join("secret.txt"), f.root.join("secret-link.txt")).unwrap();

    let entries = f.workspace.list_dir("").unwrap();
    let out = entries.iter().find(|e| e.name == "out").unwrap();
    assert!(out.symlink);
    assert_eq!(
        out.kind,
        EntryKind::Other,
        "an escaping link is not a directory to open"
    );

    assert!(matches!(
        f.workspace.list_dir("out"),
        Err(Error::PermissionDenied { .. })
    ));
    assert!(matches!(
        f.workspace.read_text("out/secret.txt"),
        Err(Error::PermissionDenied { .. })
    ));
    assert!(matches!(
        f.workspace.read_text("secret-link.txt"),
        Err(Error::PermissionDenied { .. })
    ));
    assert!(
        f.workspace
            .write_text("secret-link.txt", "pwned", None)
            .is_err()
    );
    assert_eq!(
        fs::read_to_string(f.outside.join("secret.txt")).unwrap(),
        "top secret"
    );
}

#[test]
fn symlinks_inside_the_workspace_behave_like_their_targets() {
    let f = fixture();
    symlink("src", f.root.join("source")).unwrap();
    let entries = f.workspace.list_dir("").unwrap();
    let link = entries.iter().find(|e| e.name == "source").unwrap();
    assert_eq!(link.kind, EntryKind::Directory);
    assert!(link.symlink);
    assert_eq!(f.workspace.list_dir("source").unwrap()[0].name, "main.py");
}

#[test]
fn lists_files_for_quick_open() {
    let f = fixture();
    fs::create_dir_all(f.root.join(".git/objects")).unwrap();
    fs::write(f.root.join(".git/objects/abc"), "").unwrap();
    fs::create_dir(f.root.join("build")).unwrap();
    fs::write(f.root.join("build/out.o"), "").unwrap();
    fs::write(f.root.join(".gitignore"), "build/\n").unwrap();
    fs::write(f.root.join(".env"), "").unwrap();

    let list = f.workspace.list_files(100).unwrap();
    assert_eq!(
        list.paths,
        [".env", ".gitignore", "README.md", "src/main.py"]
    );
    assert!(!list.truncated);

    let limited = f.workspace.list_files(2).unwrap();
    assert_eq!(limited.paths.len(), 2);
    assert!(limited.truncated);
}

#[test]
fn reports_changes_made_on_disk() {
    let f = fixture();
    let (sender, received) = mpsc::channel();
    let _watcher = f
        .workspace
        .watch(move |event| drop(sender.send(event)))
        .unwrap();
    // FSEvents may deliver the watch's own setup first; give it a moment.
    std::thread::sleep(Duration::from_millis(300));
    while received.try_recv().is_ok() {}

    fs::write(f.root.join("src/new.py"), "x = 1\n").unwrap();

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match received.recv_timeout(remaining) {
            Ok(WorkspaceEvent::Changed { paths }) if paths.iter().any(|p| p == "src/new.py") => {
                break;
            }
            Ok(_) => continue,
            Err(e) => panic!("no change event for src/new.py: {e}"),
        }
    }
}

#[test]
fn versions_are_opaque_and_comparable() {
    let f = fixture();
    let a = f.workspace.file_version("README.md").unwrap().unwrap();
    let b = f.workspace.file_version("README.md").unwrap().unwrap();
    assert_eq!(a, b);
    assert_ne!(a, FileVersion("0.000000000:0".into()));
}

#[test]
fn reports_unreadable_files_and_directories() {
    let f = fixture();
    let locked = f.root.join("locked.txt");
    fs::write(&locked, "secret").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let sealed = f.root.join("sealed");
    fs::create_dir(&sealed).unwrap();
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o000)).unwrap();

    let read = f.workspace.read_text("locked.txt");
    let listed = f.workspace.list_dir("sealed");

    // Restore permissions so the temporary directory can be cleaned up.
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
    fs::set_permissions(&sealed, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(
        matches!(read, Err(Error::PermissionDenied { .. })),
        "{read:?}"
    );
    assert!(
        matches!(listed, Err(Error::PermissionDenied { .. })),
        "{listed:?}"
    );
}

#[test]
fn operations_cannot_escape_through_a_symlinked_directory() {
    // Security review, Phase 3: every mutating operation, not only reads, must
    // refuse a path whose directory part is a symlink leading outside.
    let f = fixture();
    symlink(&f.outside, f.root.join("out")).unwrap();

    assert!(f.workspace.create_file("out/planted.txt").is_err());
    assert!(f.workspace.create_dir("out/planted").is_err());
    assert!(
        f.workspace
            .write_text("out/secret.txt", "overwritten", None)
            .is_err()
    );
    assert!(f.workspace.rename("README.md", "out/README.md").is_err());
    assert!(f.workspace.rename("out/secret.txt", "stolen.txt").is_err());
    assert!(
        f.workspace
            .delete("out/secret.txt", Removal::Permanently)
            .is_err()
    );
    assert!(f.workspace.file_version("out/secret.txt").is_err());

    assert_eq!(
        fs::read_to_string(f.outside.join("secret.txt")).unwrap(),
        "top secret"
    );
    assert!(!f.outside.join("planted.txt").exists());
    assert!(!f.outside.join("planted").exists());
    assert!(!f.outside.join("README.md").exists());
    assert!(f.root.join("README.md").exists());
}

#[test]
fn quick_open_skips_dependency_and_build_directories() {
    let f = fixture();
    for dir in ["node_modules/pkg", "target/debug", ".venv/lib"] {
        fs::create_dir_all(f.root.join(dir)).unwrap();
        fs::write(f.root.join(dir).join("file.txt"), "").unwrap();
    }
    let list = f.workspace.list_files(100).unwrap();
    assert_eq!(list.paths, ["README.md", "src/main.py"]);
}

#[test]
fn reopening_refuses_a_path_that_now_leads_elsewhere() {
    let base = tempfile::tempdir().unwrap();
    let project = base.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let recorded = Workspace::open(&project).unwrap().root().to_owned();
    assert_eq!(Workspace::reopen(&recorded).unwrap().root(), recorded);

    // The folder is replaced by a symlink to somewhere the user never chose.
    let elsewhere = base.path().join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::remove_dir(&project).unwrap();
    std::os::unix::fs::symlink(&elsewhere, &project).unwrap();

    assert!(matches!(
        Workspace::reopen(&recorded),
        Err(Error::Moved { .. })
    ));
}
