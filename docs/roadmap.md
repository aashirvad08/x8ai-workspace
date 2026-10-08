# Roadmap

Each phase ends with a buildable, tested application and updated docs. A phase does
not start until the previous phase's acceptance criteria hold. Security criteria
are part of every phase, not a final pass (`docs/security.md`).

| Phase | Theme | Status |
| --- | --- | --- |
| 0 | Foundation | **Complete** |
| 1 | Real terminal | **Complete** |
| 2 | Workspace, files and editor | **Complete** |
| 3 | Workspace sessions: persistence, trust and layout | **Complete** |
| 4 | Agent runtime | **Complete** |
| 5 | Multi-agent workspaces | **Complete** |
| 6 | Model providers, secrets and local models | **Complete** |
| 7 | MCP layer | **Complete** |
| 8 | Catalog and skills | **Complete** |
| 9 | Review and merge | Next |
| 10 | Editor intelligence | Planned |
| 11 | Skills, templates and presets | Planned |
| 12 | Hardening, distribution and Linux | Planned |

The terminal version, `x8ai` (ADR 0020), comes in steps of its own, alongside
the phases. Each step is pushed and released before the next starts.

| Step | Theme | Status |
| --- | --- | --- |
| T1 | `x8ai`: the Welcome screen, spaces with a shell, the Homebrew tap | **Complete** |
| T2 | Split panes and tabs, the file list and `$EDITOR` | **Complete** |
| T3 | Agents: trust, the launch dialog, sessions and reviewing changes | **Complete** |
| T4 | Models, MCP, skills, the catalog and add-ons | Next |
| T5 | A background process: spaces outlive the window, and `x8ai` reattaches | Planned |

---

## Phase 0 — Foundation

**Objective.** Establish a clean, scalable, documented foundation that proves the
architecture works, without building product features.

**Major components.** A Tauri 2 app shell for macOS. A Cargo workspace (`x8ai-core`
contracts, `x8ai-desktop` host). A React and TypeScript frontend with a typed
native client. IPC contracts generated from Rust. The capability-based command
allowlist. Integration definitions for agents, model providers and MCP servers.
Architecture, roadmap, security docs and ADRs. CI.

**Acceptance criteria.**
- The app launches on macOS, and the UI shows data returned by a native command.
- Ungranted commands are rejected, and the rejection is visible in the UI.
- `pnpm check` passes: typecheck, tests, rustfmt, clippy with `-D warnings`, and
  cargo tests. `pnpm tauri build` produces an `.app`.
- The contracts can express Claude Code, OpenCode, Codex, Aider, Anthropic, OpenAI,
  Google, OpenRouter, Ollama, GitHub MCP and Playwright MCP without special cases.
- No terminal, editor or AI functionality is implemented.

## Phase 1 — Real terminal

**Objective.** A real, fast, interactive terminal as the app's primary surface.

**Major components.** A `pty` crate using `portable-pty` (ADR 0006). A native session
registry. `terminal_spawn`, `terminal_write`, `terminal_resize` and `terminal_kill`
commands. Output over a Tauri Channel with batching and backpressure. An
xterm.js view in `src/terminal/`. Multiple sessions as tabs. The login shell
detected from the user account.

**Acceptance criteria.**
- zsh and bash start as login shells, and `PATH` matches Terminal.app.
- vim, htop, less, `git add -p`, `ssh`, `python` REPL and full-screen TUIs work,
  including colors, the alternate screen, mouse reporting and truecolor.
- Ctrl+C interrupts the foreground job, and Ctrl+D sends EOF. Resizing reflows,
  and the child receives SIGWINCH.
- `yes` or a large `cat` does not freeze the UI. Memory stays bounded.
- Closing a tab or quitting the app leaves no orphaned processes (verified with
  `ps`).
- Security: the webview cannot choose the program a session runs. Session ids and
  sizes are validated. OSC 52 clipboard writes are disabled or confirmed. ADR 0006
  is accepted, or replaced with measurements.

## Phase 2 — Workspace, files and editor

**Objective.** Turn the terminal into a terminal-first workspace. A user-chosen
folder, with a file explorer and a code editor, sits above the terminal.

**Major components.**
- `crates/workspace`, which confines every file operation to the chosen root with
  `cap-std` (ADR 0009).
- A native folder picker.
- Lazy file explorer with create, rename and delete (to the Trash), refreshed by
  filesystem events.
- CodeMirror 6 editor (ADR 0008) with tabs, unsaved-state tracking, atomic
  version-checked saves and conflict handling.
- Quick open (⌘P) and a command palette (⇧⌘P).
- Terminal tabs whose new sessions start in the workspace root.
- Resizable explorer and terminal panels.
- A quit guard for unsaved changes.

**Acceptance criteria.**
- A folder opens only through the native picker. The explorer lists it one level
  at a time, and new terminals start in it while existing terminals stay put.
- Files open in tabs, edit, and save with ⌘S. The unsaved state is visible.
  Closing a dirty tab, opening another folder or quitting asks first.
- Create, rename and delete work, and delete is recoverable from the Trash.
- A change on disk reloads unmodified tabs and never overwrites unsaved edits.
  Saving over an external change is refused, with Overwrite or Revert offered.
- Paths outside the workspace, including through symlinks, are refused (tests).
- Errors (missing file, permission denied, binary file, conflicts) are shown to
  the user.
- The Phase 0 and Phase 1 tests still pass.

## Phase 3 — Workspace sessions: persistence, trust and layout

**Objective.** Make a workspace something the user returns to, not just a folder
open for the current launch.

**Major components.**
- Recent workspaces, and reopening the last one on launch (ADR 0010: two small
  JSON files, not SQLite).
- Workspace trust: untrusted by default, granted only in a native dialog, stored
  per exact folder, revocable, and queryable natively for Phase 4 (ADR 0010).
- Terminal split panes, each with its own PTY, and confirmation before closing a
  terminal or quitting while a program runs in it.
- Plain-text search across the workspace, streamed from the native side.
- Quit interception for Dock Quit, logout and shutdown (ADR 0011).
- The Phase 1–2 flaky close test: root cause found and fixed in `crates/pty`.

**Acceptance criteria.**
- Reopening the app restores the last workspace and panel sizes. Missing folders
  are shown and can be removed.
- An untrusted workspace runs nothing automatically. The trust decision is stored
  and revocable, and only the user can grant it.
- Search streams results with lines and columns, opens a result at its location,
  skips `.git`, dependency, build, ignored and binary files, and never leaves the
  workspace (tests).
- Terminals split right and down, resize, close and take focus; closing one that
  runs a program asks first.
- Quitting from the Dock with unsaved changes asks first.

**Deferred.** Restoring open editor tabs across launches (the files are small and
cheap to reopen by hand; it arrives with per-workspace state in a later phase).
Per-window scoping of native state waits until the app has more than one window.

## Phase 4 — Agent runtime

**Objective.** Run any compatible coding agent inside a workspace as a first-class
session, without coupling to one agent.

**Major components.** `crates/agents` (docs/agent-runtime.md, ADR 0012): built-in
definitions for Claude Code and OpenCode, the user's login-shell environment,
discovery on its `PATH`, and `AgentRuntime` on the PTY session substrate. Workspace
trust enforced; per-workspace agent approval (native dialog, stored in the app
data directory). An Agents view with status (not installed, installed, starting,
running, exited, failed). Agent terminal panes.

**Acceptance criteria.**
- An installed built-in agent can be started, used interactively, resized,
  interrupted and stopped, with the same code path for every agent. Claude Code
  validates it.
- No agent starts in an untrusted workspace, or without approval for that
  workspace; approval shows and pins the resolved executable and arguments;
  another workspace, executable or argument list needs approval again (tests).
- Approvals survive restarts and cannot be granted by project files (tests).
- Agents end when their terminal closes, another folder opens, trust is removed,
  or the app quits; no orphaned agents (tests, `ps`).
- No approval-bypass flags are added by default (test).

**Deferred.** Codex and Aider definitions (adding them is data). Pinning approvals
to a definition hash (catalog, Phase 8).

## Phase 5 — Multi-agent workspaces

**Objective.** Several agents at once in one project, without corrupting the
user's working tree or each other's work (docs/multi-agent.md, ADR 0013).

**Major components.** `crates/git` (the user's `git`: repository facts,
worktrees, changes). Agent sessions in the runtime: agent, workspace, working
directory, worktree, start time, state, PTY session. A linked worktree per session
under `~/.x8ai/worktrees`, on branch `agent/<agent>/<token>`. Rediscovery after a
restart. Review: changed files, commits and the diff, read-only in the editor.
Safe removal. Explicit non-Git behavior.

**Acceptance criteria.**
- Two agents run at once in different worktrees; stopping one leaves the other.
- The user's working tree, index and branch are unchanged by agent work (tests).
- Worktree paths and branch names are made and checked natively; the webview
  passes ids only (tests).
- Non-Git folders run one agent at a time and say they are not isolated.
- Trust and approval still gate every run (tests).
- Agent changes are shown as a diff and file list; nothing is merged
  automatically; removal never deletes commits.

## Phase 6 — Model providers, secrets and local models

**Objective.** Configure model providers once and use them from any compatible
agent.

**Delivered** (docs/models.md; ADRs 0014, 0015, 0016). An `x8ai-secrets` crate
(macOS Keychain, a value type that cannot be printed). An `x8ai-providers` crate:
Anthropic, OpenAI, Google, OpenRouter (a gateway) and Ollama (local), model ids from
the definition, from Ollama or from the user, non-secret `providers.json`, Ollama
detection on request. Agent adapters in `x8ai-agents` for Claude Code and OpenCode,
behind one trait; none for Codex. Model selection per session, kept with its
worktree. An explicit environment precedence rule. Approvals that pin the provider
and endpoint. A Models view and a model choice at launch.

**Acceptance criteria.**
- Keys are stored only in the Keychain, never cross IPC to the webview, and never
  appear in logs (tests plus review). **Met**; the only key crossing IPC is the one
  the user types, sent once to be saved.
- A key is present only in the environment of the agent that needs it. Shells do
  not inherit it (tested by inspecting a child's environment). **Met.**
- The UI offers only compatible providers for each agent, and the result is
  correct for all built-in agents. **Met** (Claude Code: Anthropic, OpenRouter,
  Ollama; OpenCode: all five; Codex: none).
- Revoking a key takes effect for new sessions immediately. **Met**: every run
  reads the Keychain again.
- An agent can run against a local model with no API key. **Met** for configuration
  (verified with the real Claude Code); running a model offline was not verified,
  because no model is pulled on the development machine and the app pulls none.

**Deferred from Phase 6**, deliberately:
- Connection tests and model listing for hosted providers: both are network
  requests on the user's behalf and need an HTTPS client the app does not have.
  Planned with an ADR on outbound requests, before Phase 8.
- Pulling models with progress, and hardware-aware suggestions: catalog territory
  (Phase 8).
- Other local servers (LM Studio, llama.cpp server, vLLM) and an `OLLAMA_HOST`
  other than the default.
- Verifying the OpenCode adapter against a running OpenCode.

## Phase 7 — MCP layer

**Objective.** Let users enable MCP servers per workspace and have every compatible
agent use them.

**Delivered** (docs/mcp.md; ADR 0017).
- An `x8ai-mcp` crate: a registry of servers the user adds (stdio or Streamable
  HTTP, variables by name and source, enabled, scope global, workspace or
  session), secrets in the Keychain, and per-workspace approvals pinning exactly
  what runs.
- An environment policy that keeps provider keys and every unlisted variable out
  of servers.
- Session-owned stdio processes behind a private socket and a bridge: started
  when the agent connects, with a startup timeout, bounded restarts and error
  output, stopped with the agent.
- MCP adapters for Claude Code (`--mcp-config`) and OpenCode
  (`OPENCODE_CONFIG_CONTENT`); agents without one are reported unsupported.
- An MCP tab, launch choices and per-session server state.

**Acceptance criteria.**
- Enabling a server makes it available to every compatible agent through that
  agent's own configuration mechanism, for the session only. **Met**, verified
  with the real Claude Code.
- Before first start, the user sees the exact command line, the resolved path and
  the variables passed (by name), and changes require re-approval. **Met**, except
  the tool list: showing it needs the app to act as an MCP client, which this
  phase deliberately does not do (see below).
- Repository-provided MCP configuration never starts in an untrusted workspace.
  **Met**: the app starts only servers the user registered, and agents run only in
  trusted folders (Phase 4). A project's own `.mcp.json` stays the agent's, with
  the agent's own approval.
- An ADR decides whether an MCP gateway is needed. **ADR 0017**: not now.

**Deferred from Phase 7**, deliberately:
- Built-in and catalog server definitions (GitHub, Playwright, databases): these
  need a signed, pinned remote catalog. Phase 8 deferred them (ADR 0018).
- An inspection client (`initialize`, list tools) and a "test" action: they would
  start servers or contact URLs outside a session; they need their own approval
  design.
- App-managed authentication for remote servers (OAuth, header tokens): agents
  authenticate themselves today.
- SSE, an MCP gateway, and OS sandboxing of servers (Phase 12).

## Phase 8 — Catalog and skills

**Objective.** One place to discover the agents, models, MCP servers and skills
the app knows, see what each needs, and bring them into a session, without a
second way to configure or run anything.

**Delivered** (docs/catalog.md; ADR 0018).
- An `x8ai-catalog` crate. It assembles items (agents, providers and models, MCP
  servers, skills), with stable ids, statuses, requirements and capabilities,
  from what each owning system reports. It adds presentation metadata of its own
  (publisher when known, tags, a version) that cannot configure anything. It is
  pure: no process, network, Keychain, trust or approval access (tested).
- A Catalog tab (⇧⌘K). Categories, local instant search, a status filter, and
  per-item actions that go to the owning system: open Agents, set up in Models,
  enable, disable or configure an MCP server, add or edit a skill, and choose a
  model, a server or a skill for the next launch.
- Skills: an `x8ai-skills` crate with three built-in skills and the user's own
  (`skills.json`). They are text only and refuse anything that looks like a key.
  They are scoped to every session, one folder, or chosen at launch. Claude Code
  gets them through `--append-system-prompt`. OpenCode and Codex are unsupported,
  with the reason.
- Sessions record their skills by id, version and fingerprint, alongside the
  agent, model and MCP servers, in worktree metadata too. A removed or changed
  skill stops the session with the reason. Nothing is silently upgraded or
  substituted.
- Codex as a built-in agent definition (no adapter), so its installed state comes
  from the runtime.
- Seams for a future signed remote catalog (`MetadataSource`, `SignedMetadata`,
  `Verifier`), with nothing remote implemented.

**Acceptance criteria.**
- Every item's status comes from the system that owns it, and nothing is shown
  as installed because metadata describes it. **Met** (tests; live check with the
  real Claude Code and Codex).
- Opening the catalog starts no process or server, probes nothing and makes no
  network connection. **Met** (tests; live check).
- A launch from catalog choices goes through the existing approval dialog, and
  the session shows its agent, model, MCP servers and skills. This holds after a
  restart. **Met** (tests; live check).
- Skills cannot hold secrets or change any other configuration. **Met** (tests).

**Deferred from Phase 8**, deliberately:
- Installation of any kind ("Install" and "Connect" flows), update
  notifications, and installed-software versions beyond what a system already
  reports. The app runs nothing to find a version.
- A signed remote index, pinned and reviewed MCP server definitions (GitHub,
  Playwright), and user-labelled untrusted remote entries: for a remote catalog
  phase, behind the seams above (ADR 0018).
- Skills for OpenCode (it reads instructions only from files) and Codex (no
  adapter).

## Phase 9 — Review and merge

**Objective.** Turn Phase 5's review into a decision: bring an agent's work into
the user's branch, or discard it, always by explicit user action.

**Major components.** Side-by-side and inline diff views of an agent session's
changes. Accepting or rejecting hunks or files. Merge, rebase or cherry-pick of an
agent branch into the user's branch, with conflicts shown, never automatic. Git
status and basics in the primary workspace.

**Acceptance criteria.**
- Every change an agent made can be reviewed as a diff before anything reaches the
  user's branch, and nothing reaches it without the user's action.
- Status and diff match `git` exactly on real repositories, including submodules
  and large repositories.
- Discarding an agent's work never deletes commits without an explicit
  confirmation.

## Phase 10 — Editor intelligence

**Objective.** Add language intelligence to the editor.

**Major components.** An LSP client with language servers managed as native child
processes. Diagnostics, completion and go-to-definition.

**Acceptance criteria.**
- TypeScript and Rust language servers provide diagnostics and completion. Crashed
  servers restart without losing editor state.
- Language servers run with the workspace as cwd and follow the same process
  lifecycle rules as sessions.

## Phase 11 — Skills, templates and presets

**Objective.** Share reusable capabilities and ready-made setups.

**Major components.** Resource bundles for skills, beyond the instruction skills
of Phase 8. Project templates. Workspace presets that bundle agent, provider,
model, MCP servers and skills into one-click stacks.

**Acceptance criteria.**
- A preset configures a workspace end to end (agent, provider, MCP) in one step and
  shows everything it will run and every secret it needs.
- Skills reach more agents through adapters, with no agent-specific UI code, and
  never through the user's global agent configuration.
- Templates never execute scaffolding scripts without approval.

## Phase 12 — Hardening, distribution and Linux

**Objective.** Ship safely, and broaden platform support.

**Major components.** Developer ID signing, notarization, the hardened runtime and
minimal entitlements. A signed auto-updater. Opt-in crash reporting with
scrubbing. Performance work. A sandboxing research spike (Seatbelt profiles,
containers or VMs for agent sessions). An external security review. A Linux build
(webkit2gtk) and CI job.

**Acceptance criteria.**
- Notarized DMG. The updater rejects unsigned or tampered updates (tests).
- The external review's high and critical findings are fixed.
- The Linux build passes CI and runs the terminal and an agent session end to end.
- The sandboxing decision is recorded in an ADR, whether it is adopted or not.

## Terminal step T1 — `x8ai`, the Welcome and a space's shell

**Objective.** One command installs the workspace, and it runs in the user's own
terminal: the Welcome screen, then a space with a real shell.

**Major components.** `crates/tui` (the `x8ai` command): ratatui over crossterm,
panes as `x8ai-pty` sessions parsed by `alacritty_terminal`, the Ctrl-g prefix,
the app's stores shared through its data folder. A release workflow that builds
a universal binary from a tag, and a Homebrew formula in `aashirvad08/homebrew-tap`.

**Acceptance criteria.**
- `brew install aashirvad08/tap/x8ai` installs it on Apple silicon and Intel;
  `x8ai` opens the Welcome screen in the terminal.
- `/cd` (a recent space by name or path, or a typed folder), `/new` and `/home`
  open a space with a login shell in its folder and `X8AI_SPACE` set; Ctrl-g h
  shows the Welcome while the shell keeps running; `/cd` back finds it as left.
- Recent spaces, ids and trust are the app's own.
- Quitting asks first while a program runs in a shell; every shell is hung up
  on quit.
- End-to-end test of the real binary on a PTY (`crates/tui/tests`).

## Terminal step T2 — panes, tabs, the file list and the editor

**Objective.** A space in `x8ai` holds a working session: shells side by side
and in tabs, the folder's files, and the user's editor (ADR 0021).

**Major components.** Pane trees per tab (`layout.rs`), the file list through
`x8ai-workspace` with its watcher (`files.rs`), editor tabs started as git
starts the editor, mouse reports for programs that ask, and `x8ai`'s own
selection, copied with `pbcopy`.

**Acceptance criteria.**
- Ctrl-g `|` and `-` split, the arrows and a click move between panes, dragging
  the line between them resizes both, `t` and `1`–`9` open and show tabs, `x`
  closes a pane (asking while a program runs).
- Ctrl-g `f` lists the folder, follows changes on disk, and Enter opens a file
  in `$VISUAL`/`$EDITOR`/`vi` in its own tab, which closes when the editor ends
  well.
- The wheel scrolls back; a program that asked for the mouse gets it, at
  positions inside its pane; dragging over text copies it.
- End-to-end test of the real binary: splits, focus by key and click, a dragged
  divider (`stty size`), tabs, the file list and editor, selection, the wheel.

## Terminal step T3 — agents

**Objective.** Agents in `x8ai` as in the app's Agents view: started in a
worktree of their own, only in trusted folders and once allowed there, then
watched, reviewed, run again, stopped and removed (ADR 0022).

**Major components.** The Agents panel (`app/agent_panel.rs`), `agents.rs`
over `x8ai-agents` (runtime, isolation, plans), trust and approvals through the
app's stores (`spaces.rs`), agent panes that stay when the agent ends, and the
review in `less`.

**Acceptance criteria.**
- Enter on an agent asks to trust the folder, then to allow the agent (folder,
  program, model, isolation), and runs it in a new worktree; the user's working
  tree stays clean.
- A session's changes (branch, commits, files, the diff) show in a pager; its
  agent can run again (Enter), be stopped (`s`, or closing its pane), and the
  session removed, discarding uncommitted changes only when asked and keeping a
  branch with commits.
- Removing trust takes the folder's approvals back and stops its agents.
- Approvals and worktrees are the app's own.
- End-to-end test with a stand-in agent; checked once with the real Claude
  Code (drawn, stopped, removed).

