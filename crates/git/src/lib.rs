//! Just enough Git for isolated agent workspaces (docs/multi-agent.md): facts
//! about a repository, linked worktrees, and what changed in one.
//!
//! Everything goes through the user's own `git` (ADR 0013), started directly with
//! explicit arguments, never through a shell. Every call:
//!
//! - removes `GIT_*` variables from the environment, so nothing inherited
//!   (`GIT_DIR`, `GIT_WORK_TREE`, …) can point Git somewhere else;
//! - disables hooks and the filesystem monitor (`core.hooksPath=/dev/null`,
//!   `core.fsmonitor=false`), so the app's own Git operations never run code
//!   from the repository;
//! - never prompts (`GIT_TERMINAL_PROMPT=0`, no input), and is killed with its
//!   process group if it takes too long.
//!
//! Revisions and branch names passed in are checked here: a commit must be a full
//! hexadecimal object id, so a value from a file cannot smuggle in an option.
//! No Tauri dependency.

#![forbid(unsafe_code)]

use std::io::Read;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;

/// How long a Git command may take. Checking out a worktree of a large
/// repository is the slowest thing done here.
const TIMEOUT: Duration = Duration::from_secs(120);

/// Output beyond this is cut off (and reported as truncated where it matters).
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not run git ({program}): {detail}")]
    Start { program: String, detail: String },
    #[error("git {command} failed: {message}")]
    Failed { command: String, message: String },
    #[error("git {command} did not finish within {} s", TIMEOUT.as_secs())]
    Timeout { command: String },
    #[error("{0}")]
    Invalid(String),
}

/// The user's `git`, with the environment it runs in.
#[derive(Debug, Clone)]
pub struct Git {
    program: PathBuf,
    env: Vec<(String, String)>,
}

/// A repository as seen from a directory inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    /// The working tree the directory is in (canonical).
    pub toplevel: PathBuf,
    /// The repository's shared Git directory (canonical), the same for all of its
    /// worktrees.
    pub common_dir: PathBuf,
    /// Where the directory is inside the working tree, `""` at the top, else
    /// ending in `/`.
    pub prefix: String,
    /// The checked-out commit; `None` before the first commit.
    pub head: Option<String>,
    /// The checked-out branch; `None` when detached.
    pub branch: Option<String>,
}

/// One entry of `git worktree list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: Option<String>,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
    /// New and not added to Git.
    Untracked,
    /// Anything else Git reports (type change, copy, …).
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub status: FileStatus,
    /// The old path of a rename.
    pub from: Option<String>,
}

/// What changed in a worktree since `base`: commits made there, and the
/// difference between `base` and the files as they are now (committed or not).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    pub head: String,
    pub branch: Option<String>,
    pub base: String,
    /// Commits on top of `base`.
    pub commits: u32,
    /// Some changes are not committed (untracked files included).
    pub uncommitted: bool,
    pub files: Vec<ChangedFile>,
    /// A unified diff of every change, new files included.
    pub diff: String,
    /// The diff was cut off at the size limit.
    pub truncated: bool,
}

/// Most new files whose content is added to the diff.
const MAX_UNTRACKED_IN_DIFF: usize = 100;

impl Git {
    /// Uses `program` with `env` (typically the user's login environment), minus
    /// every `GIT_*` variable.
    pub fn new(program: PathBuf, env: &[(String, String)]) -> Self {
        let mut env: Vec<(String, String)> = env
            .iter()
            .filter(|(name, _)| !name.starts_with("GIT_"))
            .cloned()
            .collect();
        env.push(("GIT_TERMINAL_PROMPT".into(), "0".into()));
        // Status and diff must not take locks that would disturb the user's own
        // Git commands, or an agent's.
        env.push(("GIT_OPTIONAL_LOCKS".into(), "0".into()));
        Self { program, env }
    }

    pub fn program(&self) -> &Path {
        &self.program
    }

    /// The repository `dir` belongs to, or `None` if it is not in one.
    pub fn repository(&self, dir: &Path) -> Result<Option<Repository>, Error> {
        let output = self.output(
            dir,
            &[
                "rev-parse",
                "--show-toplevel",
                "--git-common-dir",
                "--show-prefix",
            ],
        );
        let text = match output {
            Ok(text) => text,
            Err(Error::Failed { message, .. }) if message.contains("not a git repository") => {
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let mut lines = text.lines();
        let (Some(toplevel), Some(common_dir)) = (lines.next(), lines.next()) else {
            return Err(Error::Invalid(format!(
                "unexpected rev-parse output: {text:?}"
            )));
        };
        let prefix = lines.next().unwrap_or("").to_owned();
        let canonical = |path: &Path| {
            std::fs::canonicalize(path)
                .map_err(|e| Error::Invalid(format!("{}: {e}", path.display())))
        };
        let toplevel = canonical(Path::new(toplevel))?;
        let common_dir = canonical(&dir.join(common_dir))?;
        let head = self
            .output(dir, &["rev-parse", "--verify", "--quiet", "HEAD^{commit}"])
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let branch = self
            .output(dir, &["symbolic-ref", "--short", "--quiet", "HEAD"])
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        Ok(Some(Repository {
            toplevel,
            common_dir,
            prefix,
            head,
            branch,
        }))
    }

    /// Creates a worktree at `path` on a new branch `branch`, starting at `base`.
    /// The user's working tree is not touched.
    pub fn add_worktree(
        &self,
        repo: &Repository,
        path: &Path,
        branch: &str,
        base: &str,
    ) -> Result<(), Error> {
        check_commit(base)?;
        check_branch(branch)?;
        let path = absolute(path)?;
        self.output(
            &repo.toplevel,
            &["worktree", "add", "--quiet", "-b", branch, path, base],
        )
        .map(drop)
    }

    /// Removes the worktree at `path`. Unless `force`, Git refuses if it has
    /// uncommitted changes. A worktree whose directory is already gone is pruned.
    pub fn remove_worktree(
        &self,
        repo: &Repository,
        path: &Path,
        force: bool,
    ) -> Result<(), Error> {
        let shown = absolute(path)?;
        if !path.exists() {
            return self
                .output(&repo.toplevel, &["worktree", "prune"])
                .map(drop);
        }
        let mut args = vec!["worktree", "remove"];
        if force {
            args.push("--force");
        }
        args.push(shown);
        self.output(&repo.toplevel, &args).map(drop)
    }

    /// Deletes a branch, even if it is not merged. Callers decide whether its
    /// commits may go.
    pub fn delete_branch(&self, repo: &Repository, branch: &str) -> Result<(), Error> {
        check_branch(branch)?;
        self.output(&repo.toplevel, &["branch", "-D", branch])
            .map(drop)
    }

    pub fn branch_exists(&self, repo: &Repository, branch: &str) -> Result<bool, Error> {
        check_branch(branch)?;
        let reference = format!("refs/heads/{branch}");
        match self.output(
            &repo.toplevel,
            &["show-ref", "--verify", "--quiet", &reference],
        ) {
            Ok(_) => Ok(true),
            Err(Error::Failed { .. }) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// The repository's worktrees, the main one first.
    pub fn worktrees(&self, repo: &Repository) -> Result<Vec<WorktreeEntry>, Error> {
        let text = self.output(&repo.toplevel, &["worktree", "list", "--porcelain", "-z"])?;
        let mut entries = Vec::new();
        let mut current: Option<WorktreeEntry> = None;
        for field in text.split('\0') {
            if field.is_empty() {
                entries.extend(current.take());
            } else if let Some(path) = field.strip_prefix("worktree ") {
                entries.extend(current.take());
                current = Some(WorktreeEntry {
                    path: PathBuf::from(path),
                    head: None,
                    branch: None,
                });
            } else if let Some(entry) = current.as_mut() {
                if let Some(head) = field.strip_prefix("HEAD ") {
                    entry.head = Some(head.to_owned());
                } else if let Some(branch) = field.strip_prefix("branch refs/heads/") {
                    entry.branch = Some(branch.to_owned());
                }
            }
        }
        entries.extend(current);
        Ok(entries)
    }

    /// Whether the working tree at `dir` has no changes, untracked files included.
    pub fn is_clean(&self, dir: &Path) -> Result<bool, Error> {
        let text = self.output(
            dir,
            &["status", "--porcelain", "-z", "--untracked-files=all"],
        )?;
        Ok(text.is_empty())
    }

    /// Commits on `dir`'s HEAD that are not in `base`.
    pub fn commits_since(&self, dir: &Path, base: &str) -> Result<u32, Error> {
        check_commit(base)?;
        let range = format!("{base}..HEAD");
        let text = self.output(dir, &["rev-list", "--count", &range])?;
        text.trim()
            .parse()
            .map_err(|_| Error::Invalid(format!("unexpected rev-list output: {text:?}")))
    }

    /// Everything that changed in the worktree at `dir` since `base`.
    pub fn changes(&self, dir: &Path, base: &str) -> Result<Changes, Error> {
        check_commit(base)?;
        let head = self
            .output(dir, &["rev-parse", "--verify", "HEAD^{commit}"])?
            .trim()
            .to_owned();
        let branch = self
            .output(dir, &["symbolic-ref", "--short", "--quiet", "HEAD"])
            .ok()
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty());
        let commits = self.commits_since(dir, base)?;
        let uncommitted = !self.is_clean(dir)?;

        let names = self.output(
            dir,
            &[
                "diff",
                "--name-status",
                "-z",
                "-M",
                "--no-ext-diff",
                base,
                "--",
            ],
        )?;
        let mut files = parse_name_status(&names);
        let untracked = self.output(dir, &["ls-files", "--others", "--exclude-standard", "-z"])?;
        let untracked: Vec<&str> = untracked.split('\0').filter(|p| !p.is_empty()).collect();
        files.extend(untracked.iter().map(|path| ChangedFile {
            path: (*path).to_owned(),
            status: FileStatus::Untracked,
            from: None,
        }));

        let (mut diff, mut truncated) = self.output_limited(
            dir,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "-M",
                base,
                "--",
            ],
        )?;
        for path in untracked.iter().take(MAX_UNTRACKED_IN_DIFF) {
            if truncated {
                break;
            }
            // Exits with 1 when there is a difference, which is always here.
            let (patch, cut) = self.output_limited(
                dir,
                &[
                    "diff",
                    "--no-index",
                    "--no-ext-diff",
                    "--no-color",
                    "--",
                    "/dev/null",
                    path,
                ],
            )?;
            diff.push_str(&patch);
            truncated = cut || diff.len() > MAX_OUTPUT_BYTES;
        }
        truncated |= untracked.len() > MAX_UNTRACKED_IN_DIFF;
        Ok(Changes {
            head,
            branch,
            base: base.to_owned(),
            commits,
            uncommitted,
            files,
            diff,
            truncated,
        })
    }

    fn output(&self, dir: &Path, args: &[&str]) -> Result<String, Error> {
        let result = self.run(dir, args, MAX_OUTPUT_BYTES)?;
        if !result.success {
            return Err(Error::Failed {
                command: args.first().copied().unwrap_or_default().to_owned(),
                message: result.stderr.trim().to_owned(),
            });
        }
        Ok(result.stdout)
    }

    /// Output that may legitimately be large (a diff): cut off at the limit
    /// instead of failing. `diff --no-index` exits with 1 when files differ.
    fn output_limited(&self, dir: &Path, args: &[&str]) -> Result<(String, bool), Error> {
        let result = self.run(dir, args, MAX_OUTPUT_BYTES)?;
        let differs = args.contains(&"--no-index") && result.code == Some(1);
        if !result.success && !differs && !result.truncated {
            return Err(Error::Failed {
                command: args.first().copied().unwrap_or_default().to_owned(),
                message: result.stderr.trim().to_owned(),
            });
        }
        Ok((result.stdout, result.truncated))
    }

    fn run(&self, dir: &Path, args: &[&str], limit: usize) -> Result<Run, Error> {
        let command_name = args.first().copied().unwrap_or_default().to_owned();
        let mut child = Command::new(&self.program)
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
                "-c",
                "core.quotePath=false",
            ])
            .args(args)
            .current_dir(dir)
            .env_clear()
            .envs(self.env.iter().map(|(n, v)| (n, v)))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| Error::Start {
                program: self.program.display().to_string(),
                detail: e.to_string(),
            })?;
        let stdout = read_in_background(child.stdout.take().expect("piped"), limit);
        let stderr = read_in_background(child.stderr.take().expect("piped"), 64 * 1024);
        let deadline = Instant::now() + TIMEOUT;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                _ => {
                    if let Ok(pid) = i32::try_from(child.id()) {
                        let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
                    }
                    let _ = child.wait();
                    return Err(Error::Timeout {
                        command: command_name,
                    });
                }
            }
        };
        let (stdout, truncated) = stdout.join().unwrap_or_default();
        let (stderr, _) = stderr.join().unwrap_or_default();
        Ok(Run {
            success: status.success(),
            code: status.code(),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
            truncated,
        })
    }
}

struct Run {
    success: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
    truncated: bool,
}

fn read_in_background(
    mut pipe: impl Read + Send + 'static,
    limit: usize,
) -> std::thread::JoinHandle<(Vec<u8>, bool)> {
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let _ = pipe.by_ref().take(limit as u64).read_to_end(&mut output);
        // Anything left means the output was cut off; drain it so git can exit.
        let mut rest = [0u8; 8192];
        let mut truncated = false;
        while let Ok(n) = pipe.read(&mut rest) {
            if n == 0 {
                break;
            }
            truncated = true;
        }
        (output, truncated)
    })
}

/// `git diff --name-status -z`: a status, then one path (two for renames and
/// copies), each followed by NUL.
fn parse_name_status(text: &str) -> Vec<ChangedFile> {
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let mut files = Vec::new();
    while let Some(code) = fields.next() {
        let status = match code.chars().next() {
            Some('A') => FileStatus::Added,
            Some('M') => FileStatus::Modified,
            Some('D') => FileStatus::Deleted,
            Some('R') => FileStatus::Renamed,
            _ => FileStatus::Other,
        };
        let two_paths = matches!(code.chars().next(), Some('R' | 'C'));
        let Some(first) = fields.next() else { break };
        if two_paths {
            let Some(second) = fields.next() else { break };
            files.push(ChangedFile {
                path: second.to_owned(),
                status,
                from: Some(first.to_owned()),
            });
        } else {
            files.push(ChangedFile {
                path: first.to_owned(),
                status,
                from: None,
            });
        }
    }
    files
}

/// A full object id (SHA-1 or SHA-256), nothing else: never an option or a range.
pub fn check_commit(value: &str) -> Result<(), Error> {
    let valid = matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase());
    valid
        .then_some(())
        .ok_or_else(|| Error::Invalid(format!("{value:?} is not a commit id")))
}

/// Branch names created here: `agent/<agent id>/<token>`, lowercase letters,
/// digits and `-` in each part.
pub fn check_branch(value: &str) -> Result<(), Error> {
    let part = |p: &str| {
        !p.is_empty()
            && p.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            && !p.starts_with('-')
    };
    let parts: Vec<&str> = value.split('/').collect();
    let valid = parts.len() == 3 && parts[0] == "agent" && part(parts[1]) && part(parts[2]);
    valid
        .then_some(())
        .ok_or_else(|| Error::Invalid(format!("{value:?} is not an agent branch")))
}

fn absolute(path: &Path) -> Result<&str, Error> {
    match path.to_str() {
        Some(text) if path.is_absolute() => Ok(text),
        _ => Err(Error::Invalid(format!(
            "{} is not an absolute path",
            path.display()
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_full_object_ids_are_commits() {
        assert!(check_commit(&"a".repeat(40)).is_ok());
        assert!(check_commit(&"0123456789abcdef".repeat(4)).is_ok());
        for bad in [
            "HEAD",
            "main",
            "--output=/tmp/x",
            &"A".repeat(40),
            &"a".repeat(39),
            "a..b",
        ] {
            assert!(check_commit(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn only_agent_branches_are_accepted() {
        assert!(check_branch("agent/claude-code/20260929-101500-a1b2").is_ok());
        for bad in [
            "main",
            "agent/claude-code",
            "agent/../x",
            "agent/-x/y",
            "agent/Claude/x",
            "agent/a/b/c",
            "--force",
        ] {
            assert!(check_branch(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn parses_name_status_with_renames() {
        let files = parse_name_status("M\0src/a.rs\0R100\0old.txt\0new.txt\0D\0gone\0A\0added\0");
        assert_eq!(
            files,
            vec![
                ChangedFile {
                    path: "src/a.rs".into(),
                    status: FileStatus::Modified,
                    from: None
                },
                ChangedFile {
                    path: "new.txt".into(),
                    status: FileStatus::Renamed,
                    from: Some("old.txt".into())
                },
                ChangedFile {
                    path: "gone".into(),
                    status: FileStatus::Deleted,
                    from: None
                },
                ChangedFile {
                    path: "added".into(),
                    status: FileStatus::Added,
                    from: None
                },
            ]
        );
    }
}
