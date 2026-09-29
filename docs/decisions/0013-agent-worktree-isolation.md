# 0013. Agent isolation with Git worktrees, through the user's git

**Status:** Accepted (Phase 5)

## Context

Phase 5 runs several agents in one project at once. Agents that share the user's
working tree would overwrite each other's edits, and the user's, and their
changes could not be told apart. The phase must isolate them without inventing a
workflow for folders that are not Git repositories, and without merging anything
on the user's behalf. Four questions:

1. **Isolation mechanism.** Copies of the project, overlay filesystems, or Git
   worktrees?
2. **Git access.** The `git` CLI, or a library (`gix`, `git2`)? This was an open
   decision for a later phase in the roadmap.
3. **Location.** Where do agent worktrees live? The obvious places each have a
   problem: inside the project pollutes the user's tree; a sibling of the project
   writes outside it; inside `.git` is invisible but some agents make `.git`
   read-only in their sandbox (Codex does), and the primary workspace's watcher
   would see every change; the macOS app data directory has a space in its path,
   which breaks tools (a Python virtualenv's scripts cannot start from a path with
   a space).
4. **Trust in inputs.** Branch names, paths and revisions must not come from the
   webview or from files a repository could influence.

## Decision

**Linked Git worktrees, one per agent session.** `git worktree add -b
agent/<agent>/<token> <path> <base>` from the commit the user has checked out. A
worktree shares the repository's objects, so it is cheap and its changes are
ordinary Git changes (a branch, commits, a diff) that the user can review with any
Git tool. Folders that are not Git repositories get no isolation: one agent at a
time, stated plainly in the UI. Git is never initialized for the user.

**The user's `git` CLI** (`crates/git`), found on their login `PATH` or at
`/usr/bin/git`. It behaves exactly as the user's own Git: their version, config,
credential helpers and repository formats. Every app call starts it directly with
explicit arguments, no shell, `GIT_*` variables removed, hooks and fsmonitor
disabled (`core.hooksPath=/dev/null`, `core.fsmonitor=false`), no prompts, and a
timeout with process-group kill. Revisions passed in must be full object ids;
branches must match the `agent/<id>/<token>` form the app makes.

**Worktrees under `~/.x8ai/worktrees/<repository>-<hash>/<agent>-<token>`**, with a
small metadata file beside each. The directory is the app's (0700; a symlink is
refused), outside the project and outside `.git`, without spaces. Git's own
bookkeeping stays in the repository's `.git/worktrees/`. Every name is derived from
the agent's id and a generated token; the created path is checked to be exactly
the intended one. Metadata is trusted only when it is exactly what the app writes
and Git lists the worktree.

**Removal keeps committed work.** A worktree with uncommitted changes is removed
only when the user confirms discarding them; a branch with commits is kept; only a
branch with no commits beyond its base is deleted.

## Consequences

- Several agents, and the user, work at once without touching each other's files.
  Review is a plain Git diff against the session's base.
- Agents start from the last commit, not from the user's uncommitted changes. The
  UI says so. Carrying uncommitted changes over (a stash, a patch) is possible
  later, explicitly.
- Agent branches accumulate in the repository until removed. They are visible to
  the user's own Git tools, which is intended.
- Creating a worktree checks out the whole tree; very large repositories take as
  long as a `git worktree add` does (timeout: 120 s).
- Worktrees survive app restarts and are found again from the metadata. Deleting
  `~/.x8ai` by hand leaves Git records that `git worktree prune` (which the app
  runs when removing a missing worktree) cleans up.
- Isolation is by working directory, not a sandbox: a determined agent can still
  write elsewhere as the user.

## Alternatives considered

- **Copying the project** per agent. Slow and large for real repositories, and
  changes are hard to review or bring back. Rejected.
- **Copy-on-write clones or overlay filesystems** (APFS clones, overlayfs).
  Platform-specific, and still no reviewable history. Rejected for now.
- **`gix` or `git2`.** In-process and faster to call, but worktree support and
  config semantics differ from the user's `git` (hooks, credential helpers, new
  repository formats), and `git2` adds a C dependency. Rejected; revisit if Git
  calls become a bottleneck.
- **Worktrees inside `.git`** (the layout Git uses for its bookkeeping). Rejected:
  some agent sandboxes make `.git` read-only, and the primary workspace's watcher
  would report all agent activity.
- **Worktrees in the macOS app data directory.** Rejected for the space in
  `Application Support`.
- **Automatic merge** of agent branches. Out of scope by design: the user decides.
