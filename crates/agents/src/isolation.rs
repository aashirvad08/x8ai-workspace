//! Isolated agent workspaces: one linked Git worktree per agent session
//! (docs/multi-agent.md, ADR 0013).
//!
//! Worktrees live under one app-controlled directory, never in the user's
//! project and never inside `.git`:
//!
//! ```text
//! ~/.x8ai/worktrees/<repository>-<hash>/<agent>-<token>/        the worktree
//! ~/.x8ai/worktrees/<repository>-<hash>/<agent>-<token>.json    what it is
//! ```
//!
//! Git's own bookkeeping for each stays in the repository (`.git/worktrees/…`), as
//! for any worktree. Every name is made here from the agent's id (letters, digits
//! and `-`, checked when the definition is read) and a generated token, so nothing
//! from the webview or the repository decides where a worktree goes. Paths are
//! checked after creation to be exactly where they were meant to be.

use std::fs;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use x8ai_core::id::IntegrationId;
use x8ai_git::{Git, Repository, check_branch, check_commit};

/// Where every agent worktree lives. Deliberately free of spaces: tools break on
/// them (a Python virtualenv's scripts cannot start from a path with a space).
pub fn default_root(home: &Path) -> PathBuf {
    home.join(".x8ai").join("worktrees")
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(
        "isolated agent workspaces need a Git repository with at least one commit; commit something first"
    )]
    NoCommits,
    #[error("the agent's workspace has uncommitted changes")]
    HasChanges,
    #[error("{0}")]
    Git(#[from] x8ai_git::Error),
    #[error("{path}: {detail}")]
    Io { path: String, detail: String },
    #[error("{0}")]
    Unsafe(String),
}

/// An agent's worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    pub agent: IntegrationId,
    /// `<agent>-<token>`: the directory's name.
    pub name: String,
    pub path: PathBuf,
    /// `agent/<agent>/<token>`.
    pub branch: String,
    /// The commit it started from.
    pub base: String,
    /// When it was made, in milliseconds since the Unix epoch.
    pub created: u64,
}

/// What removing a worktree kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removal {
    /// The branch, when it has commits the user may still want. `None` when it had
    /// none and was deleted with the worktree.
    pub kept_branch: Option<String>,
    pub commits: u32,
}

/// Stored next to each worktree, so it is found again after the app restarts.
#[derive(Serialize, Deserialize)]
struct Metadata {
    version: u32,
    agent: String,
    token: String,
    base: String,
    created: u64,
}

const METADATA_VERSION: u32 = 1;

/// The place agent worktrees are made.
#[derive(Debug, Clone)]
pub struct Isolation {
    root: PathBuf,
}

impl Isolation {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The directory for one repository's agent worktrees: its folder name (for
    /// people) and a hash of its Git directory (to tell apart repositories with
    /// the same name).
    pub fn repository_dir(&self, repo: &Repository) -> PathBuf {
        let name = repo
            .toplevel
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let readable: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' || c == '_' {
                    c
                } else {
                    '-'
                }
            })
            .take(40)
            .collect();
        let readable = readable.trim_matches(['-', '.']);
        let hash = fnv1a(repo.common_dir.as_os_str().as_encoded_bytes());
        self.root.join(format!(
            "{}-{:012x}",
            if readable.is_empty() {
                "repo"
            } else {
                readable
            },
            hash & 0xffff_ffff_ffff
        ))
    }

    /// Makes a new worktree for `agent` on a new branch, starting from the commit
    /// the user has checked out. The user's working tree is not touched; changes
    /// they have not committed are not in the worktree.
    pub fn create(
        &self,
        git: &Git,
        repo: &Repository,
        agent: &IntegrationId,
    ) -> Result<Worktree, Error> {
        let base = repo.head.clone().ok_or(Error::NoCommits)?;
        let dir = self.repository_dir(repo);
        ensure_private_dir(&self.root)?;
        ensure_private_dir(&dir)?;
        for _ in 0..8 {
            let token = token();
            let worktree = Worktree {
                agent: agent.clone(),
                name: format!("{agent}-{token}"),
                path: dir.join(format!("{agent}-{token}")),
                branch: format!("agent/{agent}/{token}"),
                base: base.clone(),
                created: now_ms(),
            };
            check_branch(&worktree.branch)?;
            if worktree.path.exists() || git.branch_exists(repo, &worktree.branch)? {
                continue;
            }
            if let Err(error) = git.add_worktree(repo, &worktree.path, &worktree.branch, &base) {
                // Leave nothing half-made behind: no directory, no branch.
                let _ = git.remove_worktree(repo, &worktree.path, true);
                let _ = fs::remove_dir_all(&worktree.path);
                if git.branch_exists(repo, &worktree.branch).unwrap_or(false) {
                    let _ = git.delete_branch(repo, &worktree.branch);
                }
                return Err(error.into());
            }
            // Where it was meant to be, and nowhere else.
            let made = canonical(&worktree.path)?;
            if made.parent() != Some(canonical(&dir)?.as_path()) || !made.ends_with(&worktree.name)
            {
                let _ = git.remove_worktree(repo, &worktree.path, true);
                return Err(Error::Unsafe(format!(
                    "the worktree ended up at {}",
                    made.display()
                )));
            }
            write_metadata(
                &dir.join(format!("{}.json", worktree.name)),
                &Metadata {
                    version: METADATA_VERSION,
                    agent: agent.to_string(),
                    token,
                    base,
                    created: worktree.created,
                },
            )?;
            return Ok(Worktree {
                path: made,
                ..worktree
            });
        }
        Err(Error::Unsafe(
            "could not find an unused worktree name".into(),
        ))
    }

    /// The agent worktrees made earlier for `repo` that still exist, oldest first.
    /// Anything in the directory that is not exactly what `create` makes is
    /// ignored.
    pub fn find(&self, git: &Git, repo: &Repository) -> Result<Vec<Worktree>, Error> {
        let dir = self.repository_dir(repo);
        let Ok(entries) = fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let listed = git.worktrees(repo)?;
        let mut found: Vec<Worktree> = entries
            .flatten()
            .filter_map(|entry| {
                let name = entry
                    .file_name()
                    .to_str()?
                    .strip_suffix(".json")?
                    .to_owned();
                let bytes = fs::read(entry.path()).ok()?;
                let meta: Metadata = serde_json::from_slice(&bytes).ok()?;
                let agent = IntegrationId::new(meta.agent).ok()?;
                let valid = meta.version == METADATA_VERSION
                    && is_token(&meta.token)
                    && name == format!("{agent}-{}", meta.token)
                    && check_commit(&meta.base).is_ok();
                if !valid {
                    return None;
                }
                let path = canonical(&dir.join(&name)).ok()?;
                // Only worktrees Git knows as worktrees of this repository.
                listed
                    .iter()
                    .any(|w| canonical(&w.path).ok().as_deref() == Some(path.as_path()))
                    .then(|| Worktree {
                        branch: format!("agent/{agent}/{}", meta.token),
                        agent,
                        name,
                        path,
                        base: meta.base,
                        created: meta.created,
                    })
            })
            .collect();
        found.sort_by_key(|w| w.created);
        Ok(found)
    }

    /// Removes an agent's worktree. Refused while it has uncommitted changes,
    /// unless `discard`. Its branch is deleted only if the agent committed nothing
    /// on it; otherwise the branch, and so every commit, is kept.
    pub fn remove(
        &self,
        git: &Git,
        repo: &Repository,
        worktree: &Worktree,
        discard: bool,
    ) -> Result<Removal, Error> {
        check_branch(&worktree.branch)?;
        let dir = self.repository_dir(repo);
        // Never act on a path outside this repository's worktree directory.
        if worktree.path.parent() != Some(canonical(&dir)?.as_path()) {
            return Err(Error::Unsafe(format!(
                "{} is not an agent worktree",
                worktree.path.display()
            )));
        }
        let commits = if worktree.path.exists() {
            if !discard && !git.is_clean(&worktree.path)? {
                return Err(Error::HasChanges);
            }
            git.commits_since(&worktree.path, &worktree.base)?
        } else {
            0
        };
        git.remove_worktree(repo, &worktree.path, discard)?;
        let kept_branch = if commits > 0 {
            Some(worktree.branch.clone())
        } else {
            if git.branch_exists(repo, &worktree.branch)? {
                git.delete_branch(repo, &worktree.branch)?;
            }
            None
        };
        let _ = fs::remove_file(dir.join(format!("{}.json", worktree.name)));
        Ok(Removal {
            kept_branch,
            commits,
        })
    }
}

/// Creates `dir` (0700) if needed, and refuses one that is a symlink: it could
/// lead worktrees somewhere else.
fn ensure_private_dir(dir: &Path) -> Result<(), Error> {
    let io = |e: std::io::Error| Error::Io {
        path: dir.display().to_string(),
        detail: e.to_string(),
    };
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => Err(Error::Unsafe(format!(
            "{} is a symbolic link",
            dir.display()
        ))),
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(Error::Unsafe(format!(
            "{} is not a directory",
            dir.display()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)
            .map_err(io),
        Err(e) => Err(io(e)),
    }
}

fn write_metadata(path: &Path, metadata: &Metadata) -> Result<(), Error> {
    let io = |e: std::io::Error| Error::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    };
    let json = serde_json::to_vec_pretty(metadata).map_err(|e| Error::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })?;
    let temp = path.with_extension("json.tmp");
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)
        .map_err(io)?;
    file.write_all(&json)
        .and_then(|()| file.sync_all())
        .map_err(io)?;
    fs::rename(&temp, path).map_err(io)
}

fn canonical(path: &Path) -> Result<PathBuf, Error> {
    fs::canonicalize(path).map_err(|e| Error::Io {
        path: path.display().to_string(),
        detail: e.to_string(),
    })
}

/// `YYYYMMDD-HHMMSS-xxxxxx` (UTC): sortable, readable, and unique enough; a
/// collision is detected and another token tried.
fn token() -> String {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs();
    let (year, month, day) = civil_date(seconds / 86_400);
    let time = seconds % 86_400;
    let seed = format!(
        "{}-{}-{}",
        now.as_nanos(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}-{:06x}",
        time / 3600,
        time / 60 % 60,
        time % 60,
        fnv1a(seed.as_bytes()) & 0xff_ffff
    )
}

fn is_token(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    parts.len() == 3
        && parts[0].len() == 8
        && parts[1].len() == 6
        && parts[0]
            .bytes()
            .chain(parts[1].bytes())
            .all(|b| b.is_ascii_digit())
        && parts[2].len() == 6
        && parts[2]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn civil_date(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

/// FNV-1a, 64 bit: stable across Rust versions, unlike the standard hasher, so a
/// repository's directory name never changes.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_well_formed_and_distinct() {
        let a = token();
        let b = token();
        assert!(is_token(&a), "{a}");
        assert_ne!(a, b);
        assert!(check_branch(&format!("agent/claude-code/{a}")).is_ok());
        for bad in [
            "",
            "20260929-101500",
            "2026092-101500-abcdef",
            "20260929-101500-ABCDEF",
            "../../x",
        ] {
            assert!(!is_token(bad), "{bad}");
        }
    }

    #[test]
    fn dates_are_right() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(20_725), (2026, 9, 29));
        assert_eq!(civil_date(11_016), (2000, 2, 29));
    }

    #[test]
    fn the_hash_is_stable() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
