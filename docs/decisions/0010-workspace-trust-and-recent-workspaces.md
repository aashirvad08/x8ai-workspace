# 0010. Workspace trust and remembered workspaces

**Status:** Accepted (Phase 3)

## Context

Phase 3 makes a workspace something the user comes back to. The app has to
remember which folders were opened, reopen the last one at launch, and record
whether the user trusts a folder, which later phases need: from Phase 4 on,
agents, MCP servers and other tools must not run in a folder the user has not
trusted.

That raises four questions.

1. **What is stored, and where.** Anything stored outlives the session and could
   leak what the user works on. Storing a folder's contents, file names, or
   settings found inside it would also make the store a target for a malicious
   repository.
2. **Who may reopen a folder.** Phase 2 made the native picker the only way to
   open a folder (ADR 0009), so the webview could never name one. A recent list
   needs a second way in that keeps that property.
3. **Who may grant trust.** If the webview could grant trust, any code running in
   it, including a future XSS through file contents, could trust a folder on the
   user's behalf and so enable whatever trust enables.
4. **What trust means before anything enforces it.** No agent exists yet, and the
   phase must not add fake enforcement.

## Decision

**Two small JSON files** in the app's data directory
(`~/Library/Application Support/com.x8ai.workspace/`):
`recent-workspaces.json` and `trusted-workspaces.json`. Each is
`{ "version": 1, "workspaces": [{ "root": "/abs/path", "at": <ms since epoch> }] }`
and nothing else: no names, no file lists, no contents, nothing read from inside
the folder. The recent list keeps the 15 most recent folders, most recent first.
Only `crates/workspace` (`RecentWorkspaces`, `TrustStore`) reads or writes them,
and the webview has no command that touches them directly.

- Every change replaces the file atomically (write to a temporary file, fsync,
  rename). The file is mode 0600 and the directory 0700.
- A file that is unreadable, larger than 1 MB, not valid JSON or of an unknown
  version is moved aside to `<name>.json.corrupt` and the store starts empty. The
  user is told once (`app_take_warnings`). Nothing is silently deleted.
- Entries that are not absolute paths are dropped when the file is loaded.

**Reopening is limited to the recent list.** `workspace_open_recent(root)` opens a
folder only if it is in `recent-workspaces.json`, which only ever receives folders
the user chose in the native picker. Anything else is `permissionDenied`. So the
webview can choose among folders the user already opened, never a new one.
Recorded roots are canonical, and reopening requires the path to still resolve to
exactly that root (`Workspace::reopen`): if the folder was replaced by a symlink
to somewhere else, reopening is refused rather than opening a folder the user
never chose. A folder that no longer exists, or now leads elsewhere, is removed
from the list when reopening fails; one that is only missing (an unmounted
volume) stays listed, marked, until the user removes it or it fails to open.

**Trust is granted only in a native dialog.** `workspace_set_trust(true)` shows a
native `NSAlert` ("Trust “name”?", with the path and what trust will mean) and
changes nothing unless the user presses **Trust** there. The webview cannot answer
that dialog. Removing trust never needs native confirmation, because it only
reduces what may happen; the UI still asks, to prevent accidents.

**Trust is exact.** It applies to exactly the folder that was trusted: not its
parent, not a subfolder opened on its own, not another path to the same folder
through a symlink (roots are canonical, so a symlinked path resolves to the same
root). New folders are untrusted.

**Trust enforces nothing yet, and says so.** Today, trust is a stored decision
that is displayed (status bar) and can be granted and removed (status bar,
command palette). Nothing in the app runs automatically in any folder, trusted or
not: terminals start the user's shell only when the user opens one. The native
side answers the question in one place, `Workspaces::is_trusted(root)` in
`src-tauri/src/workspace.rs`, backed by `TrustStore::is_trusted`, and the answer
travels with the workspace (`WorkspaceInfo.trusted`). The Phase 4 agent runtime is
expected to ask it before launching anything in a workspace; until then no
interface pretends to enforce it.

## Consequences

- The recent list and trust survive restarts and app updates, and are easy to
  inspect or delete by hand.
- The files reveal which folders the user opened and trusted, which is what they
  are for. They reveal nothing about what is inside those folders.
- A second app instance writing the same files could lose an update (last writer
  wins). The app runs as a single instance, so this is accepted.
- Moving or renaming a folder loses its trust, which is the safe direction.
- Opening the most recent workspace at launch starts a login shell in it, as
  opening it by hand does. A shell may itself run code from the folder (for
  example a `direnv` hook the user installed); that is the user's shell
  configuration and is not affected by trust.

## Alternatives considered

- **SQLite.** Transactions and queries are not needed for two lists of at most a
  few dozen paths, and a database file is harder to inspect. Revisit if per-
  workspace state (tabs, layout, agent approvals) grows beyond small lists; the
  roadmap's open decision "workspace state storage" is decided in favour of JSON
  for now.
- **Security-scoped bookmarks** instead of paths. They matter for sandboxed apps,
  which this app is not (yet). Paths keep the files readable. Revisit with
  sandboxing in Phase 12.
- **Trust granted by a webview command without native confirmation.** Rejected:
  it would let anything running in the webview grant trust.
- **Inherited trust** (trusting a parent trusts everything below). Convenient, but
  it makes trusting `~` a single click that trusts every repository the user will
  ever clone. Rejected for exact matching; a deliberate "trust parent folder"
  option can be added later if needed.
- **Storing trust inside the folder** (a marker file). Rejected: the folder's own
  contents must never be able to declare it trusted.
