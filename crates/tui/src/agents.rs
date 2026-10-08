//! Agents in `x8ai`: the app's agent runtime (`x8ai-agents`), the same
//! definitions, worktrees and approvals (docs/agent-runtime.md,
//! docs/multi-agent.md). A session gets the model, MCP servers and skills
//! chosen for it (ADR 0023): the key from the Keychain, the servers through
//! `x8ai`'s own MCP runtime and bridge, the skills in its prompt.
//!
//! This module plans, creates and inspects sessions; the app asks the user and
//! runs them on panes (`app.rs`). Trust and approvals are checked through the
//! app's stores (`spaces.rs`).

use std::path::{Path, PathBuf};

use x8ai_agents::adapter::{self, AgentMcpServer, AgentMcpTransport, AgentSkill, ConfigureError};
use x8ai_agents::discovery::find_executable;
use x8ai_agents::environment::var;
use x8ai_agents::isolation::{self, Isolation};
use x8ai_agents::{AgentRuntime, AgentSession, Denied, LaunchPlan, plan};
use x8ai_core::agent::{AgentDefinition, AgentSessionId, SessionConfiguration};
use x8ai_core::id::IntegrationId;
use x8ai_core::mcp::McpServerTransport;
use x8ai_core::model::ModelSelection;
use x8ai_core::skill::{Skill, SkillRef};
use x8ai_git::{Changes, FileStatus, Git, Repository};
use x8ai_mcp::{Launch, Selection, selection};
use x8ai_workspace::TrustStore;

use crate::services::Services;

/// An agent as the Agents panel lists it.
#[derive(Debug, Clone)]
pub struct AgentRow {
    pub definition: AgentDefinition,
    /// Where its program is, or why it cannot run here (not installed).
    pub program: Result<String, String>,
    /// Allowed in the folder, with the program it would run now.
    pub approved: bool,
}

/// How agents would work in a folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolated {
    /// A Git repository with a commit: each session a worktree, from `branch`.
    Worktrees { branch: Option<String> },
    /// Not available, and why: agents run in the folder itself, one at a time.
    Shared(String),
}

pub struct Agents {
    definitions: Vec<AgentDefinition>,
    pub runtime: AgentRuntime,
    isolation: Isolation,
    /// The environment agents are found and started with.
    env: Vec<(String, String)>,
}

impl Agents {
    /// `env` is the environment agents get: `x8ai`'s own, which the user's
    /// shell set up when they started it (ADR 0022). The app reads it from a
    /// login shell instead, having none; one started from inside a terminal
    /// would stop, waiting to own it.
    pub fn new(home: &Path, env: Vec<(String, String)>) -> Self {
        Self {
            definitions: x8ai_agents::builtin(),
            runtime: AgentRuntime::default(),
            isolation: Isolation::new(isolation::default_root(home)),
            env,
        }
    }

    /// The environment agents and their tools are started with.
    pub fn env(&self) -> &[(String, String)] {
        &self.env
    }

    pub fn definitions(&self) -> &[AgentDefinition] {
        &self.definitions
    }

    pub fn definition(&self, id: &str) -> Option<&AgentDefinition> {
        self.definitions.iter().find(|d| d.id.as_str() == id)
    }

    /// The user's `git`: on the login `PATH`, or the system's.
    fn git(&self) -> Option<Git> {
        let program = find_executable("git", var(&self.env, "PATH"))
            .or_else(|| Some(PathBuf::from("/usr/bin/git")).filter(|p| p.is_file()))?;
        Some(Git::new(program, &self.env))
    }

    /// A program on the `PATH`, or at `fallback`.
    pub fn program(&self, name: &str, fallback: &str) -> PathBuf {
        find_executable(name, var(&self.env, "PATH")).unwrap_or_else(|| PathBuf::from(fallback))
    }

    /// The launch of agent `id` in `root`, with its own configuration.
    pub fn plan(&self, id: &str, root: &Path) -> Result<LaunchPlan, String> {
        let definition = self
            .definition(id)
            .ok_or_else(|| format!("There is no agent {id}."))?;
        plan(definition, &self.env, root).map_err(|denied| denied.to_string())
    }

    /// Every built-in agent: whether it is installed, and allowed in `root`
    /// (`approved` asks the app's store). Finding a program runs nothing.
    pub fn rows(
        &self,
        root: &Path,
        mut approved: impl FnMut(&LaunchPlan) -> bool,
    ) -> Vec<AgentRow> {
        self.definitions
            .iter()
            .map(|definition| {
                let planned = plan(definition, &self.env, root);
                AgentRow {
                    definition: definition.clone(),
                    approved: planned.as_ref().is_ok_and(&mut approved),
                    program: planned
                        .map(|p| p.program.display().to_string())
                        .map_err(|denied| match denied {
                            Denied::NotInstalled { .. } => "not installed".to_owned(),
                            Denied::Unsupported(_) => "not for this system".to_owned(),
                            other => other.to_string(),
                        }),
                }
            })
            .collect()
    }

    /// How agents would work in `root`.
    pub fn isolation_of(&self, root: &Path) -> Isolated {
        let Some(git) = self.git() else {
            return Isolated::Shared("Git was not found".into());
        };
        match git.repository(root) {
            Ok(Some(Repository {
                head: Some(_),
                branch,
                ..
            })) => Isolated::Worktrees { branch },
            Ok(Some(_)) => Isolated::Shared("this repository has no commits yet".into()),
            Ok(None) => Isolated::Shared("this folder is not a Git repository".into()),
            Err(error) => Isolated::Shared(format!("Git could not read this folder: {error}")),
        }
    }

    /// The agent sessions of `root`, oldest first, with the worktrees left
    /// from earlier runs (of `x8ai` or the app) found again.
    pub fn sessions_in(&self, root: &Path) -> Vec<AgentSession> {
        if let Some(git) = self.git()
            && let Ok(Some(repo)) = git.repository(root)
            && let Ok(found) = self.isolation.find(&git, &repo)
        {
            let known = self.runtime.sessions_in(root);
            let env = &self.env[..];
            for worktree in found {
                if known
                    .iter()
                    .any(|s| s.worktree.as_ref().is_some_and(|w| w.path == worktree.path))
                {
                    continue;
                }
                let agent = worktree.agent.as_str().to_owned();
                let name = self
                    .definition(&agent)
                    .map_or_else(|| agent.clone(), |d| d.name.clone());
                let configuration = SessionConfiguration::Agent {
                    shell_variables: adapter::shell_variables(&agent, env),
                };
                let cwd = cwd_in(&worktree.path, &repo);
                self.runtime
                    .adopt(&name, root, cwd, worktree, configuration);
            }
        }
        self.runtime.sessions_in(root)
    }

    /// A new session for `plan`, which must be authorized already: a worktree
    /// of its own on a new branch in a Git repository, or else the folder
    /// itself, for one agent at a time. Runs nothing.
    pub fn create(&self, plan: &LaunchPlan) -> Result<AgentSessionId, String> {
        let root = &plan.workspace;
        let git = self.git();
        let repo = match &git {
            Some(git) => git.repository(root).map_err(|e| e.to_string())?,
            None => None,
        };
        let (cwd, worktree) = match (&git, repo) {
            (Some(git), Some(repo)) => {
                let worktree = self
                    .isolation
                    .create(
                        git,
                        &repo,
                        &plan.agent,
                        plan.model.as_ref(),
                        &plan.mcp,
                        &plan.skills,
                    )
                    .map_err(|e| e.to_string())?;
                (cwd_in(&worktree.path, &repo), Some(worktree))
            }
            _ => (root.clone(), None),
        };
        self.runtime
            .create(plan, cwd, worktree)
            .map_err(|e| e.to_string())
    }

    /// The launch of agent `id` in `root`: with its own configuration, or
    /// pointed at `model` through its adapter. `Key::Read` reads the provider's
    /// key from the Keychain, to start the agent; `Key::Present` only checks
    /// one is saved, to ask approval or record a session.
    pub fn plan_with(
        &self,
        id: &str,
        root: &Path,
        model: Option<&ModelSelection>,
        services: &Services,
        key: Key,
    ) -> Result<LaunchPlan, String> {
        let plan = self.plan(id, root)?;
        let Some(model) = model else {
            return Ok(plan);
        };
        let provider = services
            .provider(model.provider.as_str())
            .ok_or_else(|| format!("There is no provider {}.", model.provider))?;
        // Checked before the key is read: an unsupported choice never touches it.
        adapter::support(id, provider).map_err(|reason| {
            ConfigureError::Unsupported {
                agent: plan.name.clone(),
                provider: provider.name.clone(),
                reason,
            }
            .to_string()
        })?;
        if key == Key::Present {
            return adapter::route(
                plan,
                provider,
                &model.model,
                services.credential_state(provider),
            )
            .map_err(|e| e.to_string());
        }
        let credential = services.credential(provider)?;
        adapter::configure(plan, provider, &model.model, credential.as_ref())
            .map_err(|e| e.to_string())
    }

    /// The MCP servers a new session of `definition` in `root` gets: the
    /// global and folder ones it can use, and the session ones `chosen`.
    pub fn mcp_new(
        &self,
        definition: &AgentDefinition,
        root: &Path,
        chosen: &[IntegrationId],
        services: &mut Services,
    ) -> Result<Selection, String> {
        let servers = services.mcp_servers();
        selection::for_new_session(
            &servers,
            &agent_mcp(definition),
            root,
            chosen,
            services.mcp_secrets(),
            var(&self.env, "PATH"),
        )
        .map_err(|e| e.to_string())
    }

    /// The MCP servers another run of a session uses: those it was made with
    /// that still exist, are on, and can run.
    pub fn mcp_run(
        &self,
        definition: &AgentDefinition,
        root: &Path,
        attached: &[IntegrationId],
        services: &mut Services,
    ) -> Selection {
        let servers = services.mcp_servers();
        selection::for_run(
            &servers,
            &agent_mcp(definition),
            root,
            attached,
            services.mcp_secrets(),
            var(&self.env, "PATH"),
        )
    }

    /// The skills a new session gets: the global and folder ones, and the
    /// session ones `chosen`. For an agent that cannot take skills, choosing
    /// one is refused and the others are not attached.
    pub fn skills_new(
        &self,
        definition: &AgentDefinition,
        root: &Path,
        chosen: &[IntegrationId],
        services: &mut Services,
    ) -> Result<Vec<Skill>, String> {
        if let Err(reason) = adapter::skills_support(definition.id.as_str()) {
            if chosen.is_empty() {
                return Ok(Vec::new());
            }
            return Err(ConfigureError::SkillsUnsupported {
                agent: definition.name.clone(),
                reason,
            }
            .to_string());
        }
        let all = services.skills();
        let attached = x8ai_skills::attach(&all, root, chosen).map_err(|e| e.to_string())?;
        Ok(attached.into_iter().cloned().collect())
    }

    /// The skills exactly as a session recorded them; refused, with the reason,
    /// when one was removed or changed since.
    pub fn skills_run(
        services: &mut Services,
        recorded: &[SkillRef],
    ) -> Result<Vec<AgentSkill>, String> {
        let all = services.skills();
        let resolved = x8ai_skills::resolve(&all, recorded).map_err(|e| format!("Skills: {e}"))?;
        Ok(resolved
            .into_iter()
            .map(|s| AgentSkill {
                reference: s.reference(),
                name: s.name.clone(),
                instructions: s.instructions.clone(),
            })
            .collect())
    }

    /// The plan with its MCP servers ready: a socket for each stdio server,
    /// where it starts when the agent connects, and the agent told where each
    /// server is, through its adapter. `x8ai` itself is the agent's bridge to
    /// a socket (`--mcp-bridge`).
    #[allow(clippy::too_many_arguments)]
    pub fn start_mcp(
        &self,
        services: &Services,
        session: AgentSessionId,
        definition: &AgentDefinition,
        plan: LaunchPlan,
        cwd: &Path,
        selection: &Selection,
        trust: &TrustStore,
        approvals: &x8ai_mcp::Approvals,
    ) -> Result<LaunchPlan, String> {
        // An earlier run of this session stops first.
        services.mcp.stop(session.0);
        let mut launches = Vec::new();
        for prepared in &selection.prepared {
            let env = x8ai_mcp::environment(&prepared.server, &self.env, services.mcp_secrets())
                .map_err(|e| format!("MCP: {e}"))?;
            launches.extend(Launch::new(prepared, env, cwd.to_owned()));
        }
        let endpoints = if launches.is_empty() {
            Vec::new()
        } else {
            let authorized =
                x8ai_mcp::authorize(&plan.workspace, &selection.prepared, trust, approvals)
                    .map_err(|e| e.to_string())?;
            services
                .mcp
                .start(session.0, &authorized, launches)
                .map_err(|e| format!("MCP: {e}"))?
        };
        let bridge =
            std::env::current_exe().map_err(|e| format!("cannot find x8ai itself: {e}"))?;
        let servers: Vec<AgentMcpServer> = selection
            .prepared
            .iter()
            .map(|prepared| AgentMcpServer {
                id: prepared.server.id.clone(),
                transport: match &prepared.server.transport {
                    McpServerTransport::StreamableHttp { url } => {
                        AgentMcpTransport::StreamableHttp { url: url.clone() }
                    }
                    McpServerTransport::Stdio { .. } => {
                        let socket = endpoints
                            .iter()
                            .find(|e| e.id == prepared.server.id)
                            .map(|e| e.socket.display().to_string())
                            .unwrap_or_default();
                        AgentMcpTransport::Stdio {
                            command: bridge.clone(),
                            args: vec![x8ai_mcp::bridge::FLAG.to_owned(), socket],
                        }
                    }
                },
            })
            .collect();
        adapter::attach_mcp(plan, &definition.capabilities.mcp_transports, &servers).map_err(|e| {
            services.mcp.stop(session.0);
            e.to_string()
        })
    }

    /// What the agent of `session` changed in its worktree since it started.
    pub fn changes(&self, session: &AgentSession) -> Result<Changes, String> {
        let worktree = session.worktree.as_ref().ok_or(
            "This agent works directly in your folder, so its changes are in your files, not a workspace of its own.",
        )?;
        let git = self.git().ok_or("Git was not found.")?;
        git.changes(&worktree.path, &worktree.base)
            .map_err(|e| e.to_string())
    }

    /// Removes a stopped session and its worktree: uncommitted changes go only
    /// with `discard`, and a branch with commits is kept. Returns the branch
    /// kept, with its commits.
    pub fn remove(
        &self,
        session: &AgentSession,
        discard: bool,
    ) -> Result<Option<(String, u32)>, String> {
        let kept = match &session.worktree {
            None => None,
            Some(worktree) => {
                let git = self.git().ok_or("Git was not found.")?;
                let repo = git
                    .repository(&session.workspace)
                    .map_err(|e| e.to_string())?
                    .ok_or("The repository is gone.")?;
                let removal = self
                    .isolation
                    .remove(&git, &repo, worktree, discard)
                    .map_err(|e| e.to_string())?;
                removal.kept_branch.map(|branch| (branch, removal.commits))
            }
        };
        self.runtime.forget(session.id).map_err(|e| e.to_string())?;
        Ok(kept)
    }
}

/// Whether a launch plan is for starting the agent (`Read`: the provider's key
/// from the Keychain) or for asking approval or recording a session
/// (`Present`: only whether one is saved). The Keychain may ask the user each
/// time a key is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Read,
    Present,
}

/// What `definition`'s agent can do with MCP servers, as its adapter says.
fn agent_mcp(definition: &AgentDefinition) -> selection::AgentMcp<'_> {
    let transports = &definition.capabilities.mcp_transports;
    selection::AgentMcp {
        name: &definition.name,
        support: adapter::mcp_support(definition.id.as_str(), transports),
        transports,
    }
}

/// Where the agent runs in a worktree: the same folder inside it as the
/// workspace is inside its repository, if the worktree has it.
fn cwd_in(worktree: &Path, repo: &Repository) -> PathBuf {
    let inside = worktree.join(repo.prefix.trim_end_matches('/'));
    if repo.prefix.is_empty() || !inside.is_dir() {
        worktree.to_owned()
    } else {
        inside
    }
}

/// The review of a session's changes, as text for a pager: what changed, then
/// the diff, colored. The diff's own control characters (a file can hold
/// anything) are shown, never sent to the terminal.
pub fn review(name: &str, changes: &Changes) -> String {
    const BOLD: &str = "\x1b[1m";
    const DIM: &str = "\x1b[2m";
    const GREEN: &str = "\x1b[32m";
    const RED: &str = "\x1b[31m";
    const CYAN: &str = "\x1b[36m";
    const RESET: &str = "\x1b[0m";
    let mut out = String::new();
    let branch = changes.branch.as_deref().unwrap_or("(no branch)");
    let base: String = changes.base.chars().take(10).collect();
    out.push_str(&format!(
        "{BOLD}{name}{RESET} · {branch} {DIM}from {base}{RESET}\n"
    ));
    let commits = match changes.commits {
        0 => "no commits".to_owned(),
        1 => "1 commit".to_owned(),
        n => format!("{n} commits"),
    };
    let uncommitted = if changes.uncommitted {
        " · changes not committed"
    } else {
        ""
    };
    out.push_str(&format!("{commits}{uncommitted}\n\n"));
    if changes.files.is_empty() {
        out.push_str("Nothing changed yet.\n");
    }
    for file in &changes.files {
        let (mark, color) = match file.status {
            FileStatus::Added => ("A", GREEN),
            FileStatus::Untracked => ("?", GREEN),
            FileStatus::Modified => ("M", CYAN),
            FileStatus::Deleted => ("D", RED),
            FileStatus::Renamed => ("R", CYAN),
            FileStatus::Other => ("·", ""),
        };
        let path = shown(&file.path);
        match &file.from {
            Some(from) => out.push_str(&format!(
                "  {color}{mark}{RESET}  {} → {path}\n",
                shown(from)
            )),
            None => out.push_str(&format!("  {color}{mark}{RESET}  {path}\n")),
        }
    }
    if !changes.diff.is_empty() {
        out.push('\n');
        for line in changes.diff.lines() {
            let line = shown(line);
            let color = if line.starts_with("diff --git") {
                BOLD
            } else if line.starts_with("@@") {
                CYAN
            } else if line.starts_with('+') && !line.starts_with("+++") {
                GREEN
            } else if line.starts_with('-') && !line.starts_with("---") {
                RED
            } else {
                ""
            };
            if color.is_empty() {
                out.push_str(&line);
            } else {
                out.push_str(&format!("{color}{line}{RESET}"));
            }
            out.push('\n');
        }
    }
    if changes.truncated {
        out.push_str(&format!(
            "\n{DIM}The diff is longer; this is its start.{RESET}\n"
        ));
    }
    out
}

/// `text` with control characters (other than tabs) shown as `^X`, so a
/// changed file cannot send escape sequences through the review.
fn shown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\t' => out.push(c),
            c if c.is_control() && (c as u32) < 0x20 => {
                out.push('^');
                out.push(char::from(b'@' + c as u8));
            }
            '\u{7f}' => out.push_str("^?"),
            c if c.is_control() => out.push('?'),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use x8ai_git::ChangedFile;

    use super::*;

    #[test]
    fn a_review_lists_the_files_and_colors_the_diff() {
        let changes = Changes {
            head: "b".repeat(40),
            branch: Some("agent/claude-code/20261008-101500-abcdef".into()),
            base: "a".repeat(40),
            commits: 1,
            uncommitted: true,
            files: vec![
                ChangedFile {
                    path: "src/new.rs".into(),
                    status: FileStatus::Untracked,
                    from: None,
                },
                ChangedFile {
                    path: "b.rs".into(),
                    status: FileStatus::Renamed,
                    from: Some("a.rs".into()),
                },
            ],
            diff: "diff --git a/x b/x\n@@ -1 +1 @@\n-old\n+new\x1b]52;c;evil\x07\n".into(),
            truncated: false,
        };
        let text = review("Claude Code", &changes);
        assert!(
            text.contains("agent/claude-code/20261008-101500-abcdef"),
            "{text}"
        );
        assert!(text.contains("1 commit · changes not committed"), "{text}");
        assert!(text.contains("?\x1b[0m  src/new.rs"), "{text}");
        assert!(text.contains("a.rs → b.rs"), "{text}");
        assert!(text.contains("\x1b[31m-old\x1b[0m"), "{text}");
        // The file's own escape sequence is shown, not sent.
        assert!(text.contains("+new^[]52;c;evil^G"), "{text}");
        assert!(!text.contains("\x1b]52"), "{text}");
    }
}
