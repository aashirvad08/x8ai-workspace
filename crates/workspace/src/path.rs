//! Workspace paths: `/`-separated paths relative to the workspace root.
//!
//! These rules are the first line of defence and produce clear errors. The second
//! line is `cap-std`, through which every operation runs: it refuses anything,
//! including symlinks, that resolves outside the root.

use std::path::Path;

use crate::Error;

/// Parses a workspace path for an entry inside the workspace. The root itself (the
/// empty path) is not an entry; see [`parse_dir`].
pub(crate) fn parse_entry(path: &str) -> Result<&Path, Error> {
    if path.is_empty() {
        return Err(Error::invalid(path, "the workspace root is not a file"));
    }
    parse(path)
}

/// Parses a workspace path naming a directory. The empty path is the root.
pub(crate) fn parse_dir(path: &str) -> Result<&Path, Error> {
    if path.is_empty() {
        return Ok(Path::new("."));
    }
    parse(path)
}

/// Checks a single file or directory name, as typed by the user.
pub(crate) fn check_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\0']) {
        return Err(Error::invalid(name, "is not a valid file name"));
    }
    Ok(())
}

fn parse(path: &str) -> Result<&Path, Error> {
    if path.starts_with('/') {
        return Err(Error::invalid(path, "must be relative to the workspace"));
    }
    if path.contains('\0') {
        return Err(Error::invalid(path, "must not contain a NUL byte"));
    }
    for component in path.split('/') {
        match component {
            "" => return Err(Error::invalid(path, "must not contain empty segments")),
            "." | ".." => {
                return Err(Error::invalid(
                    path,
                    "must not contain `.` or `..` segments",
                ));
            }
            _ => {}
        }
    }
    Ok(Path::new(path))
}

pub(crate) fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_relative_paths() {
        for ok in [
            "a",
            "src/main.rs",
            ".env",
            "dir/.hidden/file",
            "with space/ü.txt",
        ] {
            assert!(parse_entry(ok).is_ok(), "{ok:?}");
        }
        assert_eq!(parse_dir("").unwrap(), Path::new("."));
    }

    #[test]
    fn rejects_escapes_and_ambiguity() {
        for bad in [
            "",
            "/etc/passwd",
            "..",
            "../x",
            "a/../b",
            "a/./b",
            "./a",
            "a//b",
            "a/",
            "a\0b",
        ] {
            assert!(parse_entry(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn checks_names() {
        assert!(check_name("main.rs").is_ok());
        for bad in ["", ".", "..", "a/b", "a\0"] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn joins() {
        assert_eq!(join("", "c"), "c");
        assert_eq!(join("a/b", "c"), "a/b/c");
    }
}
