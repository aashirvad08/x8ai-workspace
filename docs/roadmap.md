# Roadmap

Each phase ends with a buildable, tested application and updated docs. A phase does
not start until the previous phase's acceptance criteria hold. Security criteria
are part of every phase, not a final pass (`docs/security.md`).

| Phase | Theme | Status |
| --- | --- | --- |
| 0 | Foundation | **Complete** |
| 1 | Real terminal | **Complete** |
| 2 | Workspace and projects | Next |
| 3 | Editor | Planned |
| 4 | Agent runtime | Planned |
| 5 | Secrets and model providers | Planned |
| 6 | Local models | Planned |
| 7 | MCP layer | Planned |
| 8 | Git and worktrees | Planned |
| 9 | Editor intelligence and change review | Planned |
| 10 | Catalog | Planned |
| 11 | Skills, templates and presets | Planned |
| 12 | Hardening, distribution and Linux | Planned |

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

## Phase 2 — Workspace and projects

**Objective.** Make a project folder the unit of work.

**Major components.** Open a folder as a workspace. A `workspace` crate (roots,
canonical paths, scoped file commands). A recent-workspaces list. Sessions scoped
to a workspace (cwd). Layout: split panes and tabs for terminals. A read-only file
tree. Workspace trust (untrusted by default). Persistent state (ADR: JSON vs
SQLite).

**Acceptance criteria.**
- Opening, switching and reopening workspaces restores the layout and the session
  working directories.
- File commands reject paths outside the root, including `..` and symlink escapes
  (tests).
- Untrusted workspaces run nothing automatically. The trust decision is stored and
  revocable.
- The file tree stays responsive on a 100k-file repository (lazy loading, ignore
  rules).

## Phase 3 — Editor

**Objective.** A capable basic editor that sits alongside the terminal.

**Major components.** The editor component (ADR: CodeMirror 6 vs Monaco). Open, edit
and save through workspace-scoped commands. Tabs. Syntax highlighting. In-file
search and replace. External change detection through a native file watcher.

**Acceptance criteria.**
- Multiple files open in tabs. Unsaved changes are tracked and never lost silently.
- Changes made on disk by an agent or git are detected and offered as a reload or
  diff.
- Large files (10 MB) open without freezing the UI.
- Saving outside the workspace root is impossible through the UI or IPC (tests).

## Phase 4 — Agent runtime

**Objective.** Run any compatible coding agent inside a workspace as a first-class
session, without coupling to one agent.

**Major components.** An `agents` crate. Built-in definitions for Claude Code,
OpenCode, Codex and Aider (pinned and reviewed). Installed-agent detection through
the login environment. Resolution to absolute paths. `AgentRuntime` built on the
session substrate. An agent launcher UI. Session status (running, waiting,
exited).

**Acceptance criteria.**
- Each built-in agent that is installed can be started, used interactively,
  resized, interrupted and stopped, with the same code path for all of them.
- Adding a new agent that needs no special configuration takes only a definition
  (tested with a fixture agent).
- The launch shows the resolved executable path and arguments. The first launch per
  workspace requires approval.
- No approval-bypass flags are added by default.

## Phase 5 — Secrets and model providers

**Objective.** Configure model providers once and use them from any compatible
agent.

**Major components.** A `secrets` crate (macOS Keychain). A `providers` crate with
definitions for Anthropic, OpenAI, Google and OpenRouter. Connection tests and
model listing. Agent↔provider compatibility by `ProviderApi`. Per-agent config
adapters that inject provider, model and credentials at launch.

**Acceptance criteria.**
- Keys are stored only in the Keychain, never cross IPC to the webview, and never
  appear in logs (tests plus review).
- A key is present only in the environment of the agent that needs it. Shells do
  not inherit it (tested by inspecting a child's environment).
- The UI offers only compatible providers for each agent, and the result is
  correct for all built-in agents.
- Revoking a key takes effect for new sessions immediately.

## Phase 6 — Local models

**Objective.** Make local and open-weight models a first-class choice.

**Major components.** Ollama detection and definitions. Listing installed models.
Pulling models with progress. Hardware-aware suggestions (RAM or VRAM vs model
size). Support for other OpenAI-compatible local servers (LM Studio, llama.cpp
server, vLLM).

**Acceptance criteria.**
- An agent can run against a local model with no API key and no network egress for
  model traffic. This is verified by running offline.
- Model pulls can be cancelled and resumed, and progress is shown.
- The UI clearly distinguishes local providers from remote ones.

## Phase 7 — MCP layer

**Objective.** Let users enable MCP servers per workspace and have every compatible
agent use them.

**Major components.** An `mcp` crate. Built-in definitions for GitHub, Playwright,
filesystem and a database server (pinned). Requirement checks. An inspection
client that runs `initialize` and lists tools. Per-agent MCP config generation.
Remote MCP with OAuth. The approval flow.

**Acceptance criteria.**
- Enabling a server in a workspace makes it available to every compatible agent
  through that agent's own configuration mechanism.
- Before first start, the user sees the exact command line, the resolved path, the
  secrets passed and the tool list. Changes to any of these require re-approval.
- Repository-provided MCP configuration never starts in an untrusted workspace.
- An ADR decides whether an MCP gateway is needed.

## Phase 8 — Git and worktrees

**Objective.** Git awareness, and isolated worktrees for parallel agent work.

**Major components.** A `git` crate (ADR: `git` CLI vs `gix`). Status, branches, diff
and commit basics. Worktree creation per agent session. Cleanup.

**Acceptance criteria.**
- Status and diff match `git` exactly on real repositories, including submodules
  and large repositories.
- An agent session can run in a fresh worktree on a new branch, and several can
  run in parallel without interfering.
- Worktree removal never deletes uncommitted work without an explicit
  confirmation.

## Phase 9 — Editor intelligence and change review

**Objective.** Review and steer what agents change. Add language intelligence.

**Major components.** Diff views (side-by-side and inline). Reviewing agent changes
per worktree or branch, accepting or rejecting hunks. An LSP client with language
servers managed as native child processes. Diagnostics, completion and
go-to-definition.

**Acceptance criteria.**
- Every change an agent made in a session can be reviewed as a diff before it is
  merged into the main checkout.
- TypeScript and Rust language servers provide diagnostics and completion. Crashed
  servers restart without losing editor state.
- Language servers run with the workspace as cwd and follow the same process
  lifecycle rules as sessions.

## Phase 10 — Catalog

**Objective.** Discover, install and connect agents, providers and MCP servers from
one place.

**Major components.** A `catalog` crate. A catalog entry format (a definition plus
version, source, integrity and license) with a `schemaVersion`. A signed remote
index. Built-in and user-defined sources. Browse and search UI. Install and
Connect flows. Update notifications.

**Acceptance criteria.**
- "Install" shows the exact commands, runs them in a visible session after
  approval, and registers the definition. "Connect" detects existing installs.
- The index signature is verified, and tampered entries are rejected (tests).
- Approved definitions are pinned by hash. A changed definition requires
  re-approval.
- User-defined entries are labelled untrusted.

## Phase 11 — Skills, templates and presets

**Objective.** Share reusable capabilities and ready-made setups.

**Major components.** Skill definitions (for example instruction and resource
bundles that agents can load) as a catalog kind. Project templates. Workspace
presets that bundle agent, provider, model and MCP servers into one-click stacks.

**Acceptance criteria.**
- A preset configures a workspace end to end (agent, provider, MCP) in one step and
  shows everything it will run and every secret it needs.
- Skills install into the agent-specific locations through adapters, with no
  agent-specific UI code.
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
