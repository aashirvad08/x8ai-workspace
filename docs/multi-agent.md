# Multi-agent workspaces

Several agents can work in one project at the same time without touching the
user's working tree, or each other's work. In a Git repository each agent session
gets a linked worktree of its own on a new branch. The user reviews what an agent
changed and decides what happens to it; nothing is merged automatically.

Built in Phase 5, on the agent runtime (docs/agent-runtime.md). Decisions:
ADR 0013 (worktree isolation), ADR 0012 (runtime and approval), ADR 0010 (trust).

## Session model

An **agent session** is where an agent works and the agent running there. It
outlives the agent's process: stopping or restarting the agent keeps the session
and its worktree.

| Field | Meaning |
| --- | --- |
| `id` | `AgentSessionId`, native, for the app's lifetime |
| `agent` | The agent definition's id (`claude-code`) |
| `workspace` | The workspace root the session belongs to, and was approved for |
| `cwd` | Where the agent runs: its worktree (the same subfolder as the workspace, if the workspace is a subfolder of the repository), or the workspace itself |
| `worktree` | Branch, base commit and path; `None` when not isolated |
| `startedAt` | When the session was created |
| `state` | `notRunning`, `running`, `exited` (with exit code or signal), `failed` (with the reason) |
| `terminal` | The PTY session while the agent runs |

Native: `crates/agents/src/runtime.rs` (`AgentRuntime`: `create`, `run`, `stop`,
`forget`, `adopt`), `crates/agents/src/isolation.rs` (worktrees),
`crates/git` (the user's `git`). IPC: `src-tauri/src/agents.rs`.

| Command | Does |
| --- | --- |
| `agent_create_session(agent)` | Checks trust and approval, then makes a worktree (or claims the folder) and a session. Runs nothing. |
| `agent_run(session, size, events)` | Checks trust and approval again, then runs the agent on a new PTY in the session's directory. Restart is the same call. |
| `agent_sessions()` | The open workspace's sessions, including worktrees from earlier runs |
| `agent_stop(session)` | Hangs up the agent; the session and worktree stay |
| `agent_changes(session)` | Files changed, commits, and the diff since the session's base |
| `agent_read_file(session, path)` | A file in the agent's worktree, read-only |
| `agent_remove(session, discard)` | Removes a stopped session and its worktree (below) |

The webview names sessions and agents by id only. It never supplies a path, a
branch name or a revision.

## Worktree model

```
user's project (primary workspace)            app-controlled worktree root
~/code/project/                               ~/.x8ai/worktrees/
  .git/                                         project-3b93f05788ee/
    worktrees/                                    claude-code-20260929-061527-35d38f/    ← a worktree
      claude-code-20260929-061527-35d38f/  ←─┐    claude-code-20260929-061527-35d38f.json
      opencode-20260929-061530-a1b2c3/        │    opencode-20260929-061530-a1b2c3/
  src/ …                                       └── Git's bookkeeping for each worktree
```

- **Where:** `~/.x8ai/worktrees/<repository folder>-<hash>/<agent>-<token>`. The
  hash (FNV-1a of the repository's Git directory) tells apart repositories with
  the same folder name. Git's own records for each worktree stay in the
  repository's `.git/worktrees/`, as for any worktree. The path has no spaces
  (tools break on them) and is not inside `.git` (some agents, such as Codex, make
  `.git` read-only in their sandbox, and the primary workspace's watcher would
  see every change).
- **Branch:** `agent/<agent>/<token>`, token `YYYYMMDD-HHMMSS-xxxxxx` (UTC and a
  random suffix), made natively and checked (`check_branch`). A collision with an
  existing branch or directory picks another token.
- **Base:** the commit checked out in the user's working tree when the session is
  created. The user's uncommitted changes are not in the agent's worktree; the
  Agents view says so.
- **Metadata:** `<name>.json` next to each worktree: agent, token, base commit,
  creation time. It is how sessions are found again after the app restarts. Only
  entries whose name, agent id, token and commit are exactly what the app makes,
  and that Git lists as worktrees of this repository, are used.
- **What changes in the repository:** a branch per session and Git's worktree
  records. The user's working tree, index and checked-out branch are never
  touched.

## Git requirements

- The user's `git` (found on their login `PATH`, or `/usr/bin/git`).
- The workspace must be inside a Git repository with at least one commit. A
  repository without commits is refused with a message: commit first.
- The app's own Git commands run with hooks and the filesystem monitor disabled,
  no inherited `GIT_*` variables, no prompts, and a timeout. Agents themselves use
  Git as they normally would inside their worktree.

## Non-Git workspaces

No fake isolation, and Git is never initialized for the user:

- The Agents view says the folder is not a Git repository and that agents are not
  isolated there.
- One agent at a time runs directly in the folder, as in Phase 4. Creating or
  running a second while one runs is refused (`SharedBusy`), with that reason.
- Such a session has no changes view (there is no separate workspace to compare);
  its effects are in the user's folder.

## Lifecycle

```
Launch ─▶ trust? ─▶ approval? ─▶ Git repo with a commit? ─yes─▶ new worktree + branch ─┐
                                        │no                                            ▼
                                        └─▶ the folder itself (if no agent runs there) ─▶ session (notRunning)
session ─▶ agent_run ─▶ running ─exit─▶ exited / failed ─Enter or Restart─▶ running …
running ─Stop / close terminal (asks) ─▶ exited      (the worktree stays)
not running ─Remove (asks)─▶ worktree removed; branch deleted if it has no commits
```

| Event | Agent process | Session and worktree |
| --- | --- | --- |
| Agent exits or crashes | Gone; exit reported | Stay; restartable |
| Stop, or its terminal closes (asks first) | SIGHUP, SIGKILL after 2 s | Stay |
| Another folder opens (asks first) | Stopped | Forgotten by the app for now; found again when the folder reopens |
| Folder loses trust | Stopped | Stay; running needs trust again |
| App quits (asks first) or crashes | Stopped (hangup; the kernel on a crash) | Stay on disk; found again next time |
| Remove | Must be stopped | Worktree deleted; see below |

**Removing** a session's worktree is always the user's explicit action, after a
dialog that says what goes:

- uncommitted changes are discarded only if the user confirms (`discard`);
  otherwise removal is refused;
- if the agent committed anything, its branch is kept with those commits and the
  dialog names it; only a branch with no commits beyond the base is deleted.

Nothing the agent committed is ever deleted by the app.

## Review model

For a session with a worktree, **Changes** shows, read-only:

- the branch and the number of commits the agent made;
- every changed file (added, modified, deleted, renamed, untracked), clickable;
- a unified diff of everything since the base, committed or not, new files
  included (`git diff <base>` plus new files), capped at 8 MB.

Files and the diff open as read-only editor tabs, marked as the agent's
(`/agent/<session>/…` keys, which no workspace path can have). They are read from
the worktree through a `cap-std` handle on it, with the same path checks as
workspace files. The primary workspace, its explorer and its editor tabs are
never switched to a worktree.

## Security boundaries

| Rule | How |
| --- | --- |
| Trust and approval before anything | `agent_create_session` checks both before touching Git; `agent_run` checks both again on every run |
| A session runs only in its own workspace | `agent_run` refuses a session that belongs to another workspace, or a launch for another agent (`Mismatch`) |
| No paths from the webview | Commands take ids; worktree paths, branch names and revisions are made natively and validated (`check_branch`, `check_commit`) |
| Worktrees stay in the controlled root | Names from the agent id and a generated token; the directory must not be a symlink; the created path is checked to be exactly the intended one |
| Metadata cannot redirect anything | Only well-formed entries that Git confirms as worktrees of this repository are used; paths are derived from the name, never read from the file |
| The primary working tree is untouched | Worktree creation and review only read it; tests assert it stays clean on the same commit |
| No repository code runs from the app's Git calls | Hooks and fsmonitor disabled; `GIT_*` stripped |
| Agent file reads stay in the worktree | `cap-std` handle on the worktree root |

**Honest limit:** isolation is by working directory. An agent runs as the user and
can still `cd` elsewhere or write to the primary checkout if it decides to; the
worktree prevents *accidental* interference through normal work, not a hostile
agent. The agents' own permission systems remain the control inside them, and OS
sandboxing is future work (roadmap Phase 12).

## Future merge workflow

Phase 5 stops at review. The information for the next step is already there: the
branch, its base, its commits and the diff.

```
Agent ─▶ worktree ─▶ changes ─▶ review (today) ─▶ user decision ─▶ merge / cherry-pick / discard (future)
```

A future phase adds the decision step, always an explicit user action: merge or
rebase the agent's branch into the user's branch, pick commits, or discard. The
app never applies an agent's changes to the user's branch on its own.
