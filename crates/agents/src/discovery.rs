//! Whether an agent is installed: its program, looked up on the user's `PATH`.
//!
//! Only real executable files count. Relative `PATH` entries (such as `.`) are
//! ignored, because they would resolve against the directory the lookup happens to
//! run in, which could be a workspace. The path is returned as found, without
//! resolving symlinks, so an agent that updates itself in place (a symlink to a new
//! version) is still the same program.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Finds `program` the way a shell would: an absolute path is checked as is, a
/// bare name is searched in the absolute directories of `path`, in order. Names
/// with a `/` that are not absolute are never resolved.
pub fn find_executable(program: &str, path: Option<&str>) -> Option<PathBuf> {
    if program.contains('/') {
        let candidate = Path::new(program);
        return (candidate.is_absolute() && is_executable_file(candidate))
            .then(|| candidate.to_owned());
    }
    path?
        .split(':')
        .map(Path::new)
        .filter(|dir| dir.is_absolute())
        .map(|dir| dir.join(program))
        .find(|candidate| is_executable_file(candidate))
}

fn is_executable_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn executable(dir: &Path, name: &str, mode: u32) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        path
    }

    #[test]
    fn finds_the_first_executable_on_the_path() {
        let temp = tempfile::tempdir().unwrap();
        let (first, second) = (temp.path().join("a"), temp.path().join("b"));
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        executable(&first, "tool", 0o644); // not executable: skipped
        let wanted = executable(&second, "tool", 0o755);
        fs::create_dir(first.join("dir-tool")).unwrap();

        let path = format!("{}:{}", first.display(), second.display());
        assert_eq!(find_executable("tool", Some(&path)), Some(wanted.clone()));
        assert_eq!(
            find_executable("dir-tool", Some(&path)),
            None,
            "directories are not programs"
        );
        assert_eq!(find_executable("missing", Some(&path)), None);
        assert_eq!(find_executable("tool", None), None);
        assert_eq!(
            find_executable(wanted.to_str().unwrap(), None),
            Some(wanted)
        );
    }

    #[test]
    fn ignores_relative_path_entries_and_relative_programs() {
        let temp = tempfile::tempdir().unwrap();
        executable(temp.path(), "tool", 0o755);
        // Would resolve against whatever the current directory is.
        assert_eq!(find_executable("tool", Some(".:bin:")), None);
        assert_eq!(find_executable("./tool", Some("/usr/bin")), None);
    }

    #[test]
    fn keeps_the_path_as_found_rather_than_resolving_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        let versions = temp.path().join("versions");
        fs::create_dir(&versions).unwrap();
        let real = executable(&versions, "tool-2.0", 0o755);
        let bin = temp.path().join("bin");
        fs::create_dir(&bin).unwrap();
        std::os::unix::fs::symlink(&real, bin.join("tool")).unwrap();

        let found = find_executable("tool", Some(bin.to_str().unwrap())).unwrap();
        assert_eq!(found, bin.join("tool"));
    }
}
