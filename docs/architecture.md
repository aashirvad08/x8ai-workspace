# Architecture

x8ai Workspace is a terminal-first development environment for macOS. It hosts
existing tools — shells, coding agents, model providers, MCP servers, Git — and
orchestrates them. It does not implement an LLM, and it does not implement a coding
agent. This document describes the architecture Phase 0 establishes and the shape
later phases build into.

Status markers used below: **[built]** exists in the repository today;
**[planned: Phase N]** is designed here but deliberately not implemented yet.

---

## 1. Principles

1. **Host, don't replace.** Claude Code, OpenCode, Codex, Aider and others are
   external programs the user installs. The app gives them a great environment.
2. **No privileged vendor.** Nothing in the architecture is shaped around one agent,
   one model provider or one MCP server. Compatibility is decided by declared
   protocols (§8, §9), not by names.
3. **The native side is trusted; the webview is not.** Anything that touches
   processes, files, secrets or the network happens in Rust behind a narrow,
   explicitly granted command surface (§4, `docs/security.md`).
4. **Declarative first, code where it earns its place.** Integrations are data
   (`crates/core`). Per-tool quirks that data cannot express live in small Rust
   adapters, not in the UI.
5. **Build what the phase needs.** Contracts and boundaries exist now; subsystems
   are implemented in their phase (`docs/roadmap.md`).

## 2. System overview

```
┌──────────────────────────── macOS app process (Rust) ─────────────────────────────┐
│                                                                                    │
│  src-tauri (x8ai-desktop)            crates/* (Tauri-free)                          │
│  ┌──────────────────────────┐        ┌────────────────────────────────────────┐    │
│  │ window, menu, IPC [built] │──────▶│ x8ai-core: contracts [built]           │    │
│  │ capability grants [built] │       │ x8ai-pty: sessions   [built]           │    │
│  │ sessions, workspace and   │       │ x8ai-workspace: scoped files [built]   │    │
│  │ quit guard state  [built] │       │ x8ai-agents: agent runtime   [built]   │    │
│  │                           │       │ x8ai-git: worktrees, changes [built]   │    │
│  └────────────┬─────────────┘        │ x8ai-secrets, x8ai-providers [built]   │    │
│               │                      │ x8ai-mcp: MCP servers        [built]   │    │
│               │ IPC (commands,       │ x8ai-catalog, x8ai-skills    [built]   │    │
│               │ channels, events)    │ merge and review    [planned: Phase 9] │    │
│               │                      └────────────────────────────────────────┘    │
└───────────────┼────────────────────────────────────────────────────────────────────┘
                │                                   │ spawns, owns, kills
┌───────────────▼──────────────┐     ┌──────────────▼────────────────────────────────┐
│ WKWebView (WebKit processes) │     │ child processes (user privileges)             │
│ src/ — React + TypeScript    │     │ login shells · agents · stdio MCP servers ·   │
│ UI only; no direct OS access │     │ language servers · git                        │
└──────────────────────────────┘     └───────────────────────────────────────────────┘
```

## 3. Repository map

| Path | What it is | Rule |
| --- | --- | --- |
| `crates/core/` | `x8ai-core`: IPC contracts and integration definitions | No Tauri, no I/O, no process management. Compiles and tests on any OS. |
| `crates/pty/` | `x8ai-pty`: PTY sessions, the terminal and agent substrate | No Tauri. Owns every terminal process the app starts. |
| `crates/workspace/` | `x8ai-workspace`: the chosen folder, and every file operation inside it; what is remembered about workspaces (recent, trust, agent approvals) | No Tauri. All access goes through a `cap-std` handle on the root (ADR 0009). |
| `crates/agents/` | `x8ai-agents`: built-in agent definitions, login environment, discovery, agent sessions and worktree isolation, the agent runtime (docs/agent-runtime.md, docs/multi-agent.md) | No Tauri. Starts agents only on `x8ai-pty` sessions, only when authorized. |
| `crates/git/` | `x8ai-git`: the user's `git` for repository facts, worktrees and changes (ADR 0013) | No Tauri. Explicit arguments, no shell, no hooks, no inherited `GIT_*`. |
| `crates/secrets/` | `x8ai-secrets`: provider credentials in the macOS Keychain, and `SecretValue`, which cannot be printed or serialized (ADR 0014) | No Tauri. Nothing else reads or writes the Keychain. |
| `crates/providers/` | `x8ai-providers`: built-in provider definitions, model lists, the non-secret `providers.json`, Ollama detection (docs/models.md, ADR 0016) | No Tauri, nothing about agents, no network beyond the loopback address. |
| `crates/mcp/` | `x8ai-mcp`: the MCP server registry, approvals, environment policy, session-owned stdio servers and the bridge (docs/mcp.md, ADR 0017) | No Tauri. Never speaks MCP, never contacts a server, never uses a shell. |
| `crates/skills/` | `x8ai-skills`: built-in and user skills (instructions for agent sessions), `skills.json`, which skills a session gets, and resolving a session's recorded skills (docs/catalog.md) | No Tauri. Text only: runs nothing, holds no secret. |
| `crates/catalog/` | `x8ai-catalog`: catalog metadata and items assembled from what each owning system reports (docs/catalog.md, ADR 0018) | No Tauri, and only `x8ai-core` among the app's crates: no process, network, Keychain, trust or approval access (tested). |
| `crates/addons/` | `x8ai-addons`: the built-in add-ons, what each needs and how it installs, and the zsh setup that turns them on in one space's terminals (ADR 0019) | No Tauri; only `x8ai-core` among the app's crates. Runs nothing: the desktop host runs a confirmed install, and starts terminals with the setup. |
| `crates/tui/` | `x8ai`: the terminal host, the `x8ai` command (ADR 0020). The Welcome screen and spaces with tabs of split panes, the file list and the user's editor (ADR 0021), agents through `x8ai-agents` with the app's stores (ADR 0022), and models, MCP servers, skills, the catalog and add-ons through their crates, the app's stores and Keychain items (ADR 0023), drawn with ratatui; each pane is an `x8ai-pty` session parsed by `alacritty_terminal` | No Tauri, no webview. Thin like `src-tauri`: wiring, keys and drawing; rules both hosts need live in the crates. Shares the app's data folder and stores. |
| `src-tauri/` | `x8ai-desktop`: the Tauri host | The only crate that depends on Tauri. Thin: wiring, commands, state. |
| `src-tauri/capabilities/` | Which window may call which command | Every grant is explicit and reviewed. |
| `src-tauri/permissions/autogenerated/` | Per-command `allow-*`/`deny-*` permissions generated by `tauri-build` | Committed, so permission changes show in review. CI fails on drift. |
| `src/app/` | Composition root's UI: layout, sidebar, splitters, overlays, shortcuts, status bar | UI only. No business logic, no Tauri imports. |
| `src/workbench/` | `Workbench`: every user action and the coordination between explorer, editor, terminals and search; recent folders and trust; notifications, dialogs, layout, commands | Plain TypeScript, no React. |
| `src/workspace/` | `Explorer` store (lazy tree) and `FileExplorer` view (with the recent list); `Search` store and `SearchView` | No file logic: listing, changes and search go through the native client. |
| `src/editor/` | `EditorStore` (tabs, editor states, dirty and disk state), CodeMirror setup, `EditorArea` and `EditorPane` views | The only module that may import `@codemirror/*`. |
| `src/terminal/` | `TerminalSession` (session controller), `Terminals` (tabs and split panes, shell or agent), pane trees (`panes.ts`), `TerminalView` (xterm.js), `TerminalPanel` | The only module that may import `@xterm/*`. No process logic. |
| `src/agents/` | `Agents` store (agents and agent sessions, as the native side reports them), `LaunchDrafts` (the next launch's model, servers and skills) and `AgentsView` (launch, sessions, review) | No process logic: agents are started natively; their terminals are `src/terminal/` panes; their files open read-only in the editor. |
| `src/models/` | `Providers` store (providers, key state, models) and `ModelsView` (keys, local availability, model ids) | Never holds a saved key: one is sent once to be saved, and the field is cleared. |
| `src/mcp/` | `McpServers` store and `McpView` (servers, secrets, add and edit) | Never holds a saved secret, never starts a server. |
| `src/skills/` | `Skills` store and the skills a launch can choose | Text only. |
| `src/catalog/` | `Catalog` store, local search and filters, and `CatalogView` | Read-only: every action is a `CatalogActions` request the workbench sends to the owning system. |
| `src/addons/` | `Addons` store, `/share` matching, and `AddonsView` (⇧⌘X) | Names add-ons and spaces by id; an install runs in a terminal pane from a token the native side gave after the user confirmed. |
| `src/home/` | The Welcome screen (the app's head): `Home` store, `/cd`, `/home` and `/name`, `HomeView` | Opens and closes folders only through the workbench; a folder that is not a recent one is chosen in the native picker. |
| `src/lib/` | `Store` (observable state), `useStore`, workspace path helpers | Shared building blocks. |
| `src/native/` | Typed client for native commands, error normalization | The only frontend module that may import `@tauri-apps/*`. |
| `src/contracts/generated/` | TypeScript types generated from `x8ai-core` | Never edited by hand. Regenerate with `pnpm contracts`. |
| `src/architecture.test.ts` | Tests that enforce the frontend module boundaries | Update deliberately when boundaries change. |
| `docs/decisions/` | Architecture decision records | One per significant decision. |

Subsystem modules are created in the phase that fills them rather than as empty
folders. The planned homes are:

| Concept | Frontend (`src/`) | Native (`crates/` unless noted) | Phase |
| --- | --- | --- | --- |
| core | `native/`, `contracts/` | `core` | 0 [built] |
| terminal | `terminal/` (xterm.js view, session controller) | `pty`, with commands in `src-tauri/src/terminal.rs` | 1 [built] |
| workspace | `workspace/` (explorer, search), `workbench/` | `workspace` (root, scoped files, watching, search, recent and trust stores) | 2, 3 [built] |
| editor | `editor/` | uses `workspace` file commands | 2 [built]; intelligence 9 |
| agents | `agents/` (launcher UI) | `agents` (definitions, discovery, runtime) | 4 [built] |
| models | `models/` (provider settings UI) | `providers`, `secrets`; adapters in `agents` | 6 [built] |
| mcp | `mcp/` | `mcp`; adapters in `agents` | 7 [built] |
| git | `agents/` (review), later `git/` | `git` | 5 [built: worktrees, changes]; merge later |
| catalog, skills | `catalog/`, `skills/` | `catalog`, `skills`; skill adapters in `agents` | 8 [built]; remote catalog later |

Frontend dependency direction: `app/` → `workbench/` → feature modules
(`workspace/`, `editor/`, `terminal/`, `agents/`, `models/`, `mcp/`, `skills/`,
`catalog/`) → `native/`, `contracts/` and `lib/`. `agents/` reads agent panes'
state from `terminal/`, because a running agent is a terminal pane, and model,
MCP and skill choices from `models/`, `mcp/` and `skills/`. Feature
components receive their store and an actions interface (`ExplorerActions`,
`SearchActions`, `EditorActions`, `TerminalActions`, `AgentActions`, `ModelActions`, `McpActions`, `CatalogActions`) that the workbench implements,
so they never depend on the workbench itself. `src/architecture.test.ts` enforces
the following:
- nothing depends on `app/` except `main.tsx`;
- only `native/` imports Tauri, only `terminal/` imports xterm.js, and only
  `editor/` imports CodeMirror;
- stores and the workbench never import React;
- the main window is granted exactly the commands the native client calls.

Native dependency direction: `src-tauri` → feature crates → `x8ai-core`. Feature
crates never depend on Tauri, which keeps them testable without a webview and
reusable from another host: `crates/tui` (`x8ai`, ADR 0020) is the second one,
and depends on them the same way.

## 4. Frontend/native boundary [built]

The webview renders UI and holds view state. The native host owns everything with
side effects. They communicate only through Tauri IPC.

**Mechanisms**

| Mechanism | Direction | Use | Status |
| --- | --- | --- | --- |
| Commands (`invoke`) | webview → native → reply | Request/response: queries, actions | [built] `get_app_info`, `app_*`, `terminal_*`, `workspace_*` |
| Raw-body commands | webview → native, bytes | Input that must arrive byte-exact without JSON encoding | [built] `terminal_write` |
| Channels (`tauri::ipc::Channel`) | native → webview, ordered stream | Streams and notifications: PTY output, workspace changes, search results, app events | [built] one per terminal session, one per workspace, one per search, one for app events |
| Events | native → webview, broadcast | Low-rate notifications: session exited, file changed | [planned: as needed] |

**Contracts.** Every type that crosses IPC is defined once in `x8ai-core` with serde
and exported to TypeScript with `ts-rs` (ADR 0003). The generated files are
committed. CI regenerates them and fails if they differ from the committed copy.

**Errors.** Commands return `Result<T, CommandError>`, serialized as
`{ code, message }` with `code ∈ invalidInput | notFound | permissionDenied |
internal`. The frontend's `NativeError` adds `ipc` for calls that never reached a
command: not registered, not granted, bad arguments, or no Tauri bridge. Errors
are shown to the user. They are never swallowed.

**Grants.** `src-tauri/build.rs` declares every app command, which makes Tauri
generate `allow-*` and `deny-*` permissions with no default grant. A command runs
only if a capability in `src-tauri/capabilities/` grants it to the calling window.
Otherwise Tauri rejects the call before the command's code runs. This was verified by
revoking the grant and observing `Command get_app_info not allowed by ACL`.

### Adding a native command

1. If the command introduces new request or response types, define them in the
   appropriate crate (`x8ai-core` for contracts) with `#[derive(Serialize,
   Deserialize, TS)]` and `#[ts(export)]`. Then run `pnpm contracts`.
2. Implement the command in `src-tauri/src/commands.rs` (or a submodule). Keep it
   thin: validate the untrusted arguments, then call into a crate.
3. Register it in `generate_handler!` in `src-tauri/src/lib.rs`.
4. Add its name to `COMMANDS` in `src-tauri/build.rs`. The next build regenerates
   `src-tauri/permissions/autogenerated/`. Commit that change.
5. Grant `allow-<command-name>` in the narrowest capability that needs it.
6. Add a typed method to `NativeClient` in `src/native/client.ts`, with a test.

## 5. Process model

- **App process (Rust).** One per app instance. It runs the Tauri event loop, owns
  all native state, and is the parent of every child process the app starts.
- **Webview.** WKWebView runs page content in WebKit's own sandboxed helper
  processes. From the app's point of view it is an untrusted client (ADR 0005).
- **Child processes** [built for terminal sessions and agents; other kinds later].
  These include login shells, agents, stdio MCP servers, language servers and git.
  Each is:
  - started only by the native layer, from an explicit program, arguments and
    environment. Nothing is passed through `sh -c`, and the webview never
    supplies a program path.
  - placed in its own process group (session leader for PTYs), so signals reach
    the whole tree and cleanup kills grandchildren too.
  - tracked in a registry with an id, status and exit code, and killed (SIGHUP,
    then SIGKILL after a grace period) when its session closes or the app quits.
    Orphans are a bug.
- **Environment.** A GUI app started from Finder inherits launchd's minimal
  environment: no `LANG`, and not the user's shell `PATH`. Terminal sessions
  inherit the app's environment plus `TERM`, `COLORTERM`, `TERM_PROGRAM`, and a
  `LANG` derived from the macOS language and region when nothing names a locale.
  Shells start as login shells, which rebuild the user's `PATH` from their
  profile (verified under a scrubbed environment) [built]. Agents are started
  directly (no shell) with the user's interactive login-shell environment, read
  once in the home directory, and resolved to an absolute path on its `PATH`
  [built: Phase 4, docs/agent-runtime.md]. Stdio MCP servers are started directly
  too, by the app for an agent session, with an environment it builds (a base,
  the server's listed variables, its Keychain secrets, nothing else) [built:
  Phase 7, docs/mcp.md]. App-held secrets are never placed in the app's
  own environment, so they cannot leak by inheritance: a provider key is added
  only to the environment of the agent session it was chosen for [built: Phase 6,
  docs/models.md]. Under `pnpm tauri dev`,
  sessions also inherit the dev server's environment.

## 6. Terminal architecture [built: Phase 1]

The terminal is the substrate for everything interactive, agents included. The
stack is `portable-pty` in Rust, xterm.js in the webview, and one Tauri Channel per
session (ADR 0006).

```
xterm.js ─ onData/onBinary ─▶ terminal_write (raw body) ─▶ writer thread ─▶ PTY ─▶ shell
xterm.js ◀─ bytes + events ─ Channel ◀─ sender thread ◀─ reader thread ◀─ PTY ◀────┘
   └─ rendered N bytes ─▶ terminal_ack ─▶ reader resumes when the window has room
fit ─▶ terminal_resize ─▶ TIOCSWINSZ (the foreground process gets SIGWINCH)
```

**Native (`crates/pty`).**

- `Sessions` is a registry of `Session`s keyed by `SessionId`. The webview holds
  only ids, and many sessions can run at once.
- `Program::LoginShell` runs the user's default shell as a login shell in their home
  directory. `Program::Exec` runs a specific executable without a shell, with the
  app's environment or an exact one (the agent runtime, and tests). The webview can
  request only a login shell, or an approved agent by id (§8).
- Each session has four threads: **reader** (PTY → pending buffer), **sender**
  (pending buffer and events → `SessionEvents`, in order), **writer** (queued input
  → PTY), and **waiter** (process exit). No IPC command ever blocks on the PTY.
- **Flow control:** output delivered but not yet acknowledged is capped at
  `FLOW_WINDOW` (512 KiB). When the cap is reached the reader stops, the kernel
  buffer fills, and the program blocks on write. The frontend acknowledges every
  `ACK_BYTES` (64 KiB) of rendered output. Batches are coalesced at least 4 ms
  apart. Memory per session is bounded.
- **Exit is reported after the last output:** when the PTY reports end of output,
  or, when a background job keeps the terminal open, once the reader has been
  idle for 500 ms after the exit *and* the PTY has nothing waiting to be read
  (checked with a zero-timeout `select` through `filedescriptor`, portable-pty's
  own dependency, without unsafe code), so a reader slow to be scheduled never
  counts as idle.
- **The slave side stays open in the app until the process exits.** Then the
  app closes it. On macOS, output not yet read is discarded when the exiting
  session leader's close is the terminal's last one. A short-lived program's
  output was lost whenever the reader had not run before the program exited, on
  a loaded machine or CI runner. With the app's close the last one, the kernel
  keeps the output until it is read, and end of output follows
  (`short_lived_output_survives_a_starved_reader`).
- **Close** sends SIGHUP to the shell and the foreground job, then SIGKILL to the
  shell's process group after 2 s. Jobs the user detached deliberately (`nohup`,
  `disown`) survive, as in any terminal. A closed session's output is no longer
  delivered, but the reader keeps reading (and discarding) until the end of
  output: macOS makes the last process to close a terminal wait until its unread
  output drains, so a shell that printed anything after the hangup would otherwise
  hang while exiting, beyond the reach of SIGKILL, for as long as the session was
  held. (This caused the intermittent close-test failure seen in Phases 1–2.)
- **Busy:** `Session::has_foreground_job` compares the terminal's foreground process
  group (`tcgetpgrp` on the master) with the shell's. An idle shell at its prompt
  has none; `vim`, a build or `sleep` has one. Closing and quitting ask first when
  it is true.
- **Page reload** closes every session. **App exit** hangs up all sessions and
  kills survivors after 500 ms. If the app crashes, the kernel closes the PTY and
  hangs up the session. All three paths were verified to leave no processes behind.

**Commands (`src-tauri/src/terminal.rs`)**: `terminal_create(size, events)` →
`TerminalInfo { id, program, ackBytes }`; `terminal_write` (raw body plus the
`x8ai-session-id` header); `terminal_resize(id, size)`; `terminal_ack(id, bytes)`;
`terminal_is_busy(id)`; `terminal_close(id)`. Sizes are validated (1–4096 columns,
1–2048 rows), and unknown ids are `notFound`.

**Frontend.** `TerminalSession` (`src/terminal/session.ts`) is a plain class that
connects any `TerminalScreen` (xterm.js in practice, a fake in tests) to the native
client. It queues input typed before the shell is ready, acknowledges rendered
output, forwards size changes, shows exit notices, and restarts the shell on
Enter. `TerminalView` only creates xterm.js and its add-ons (fit, WebGL, Unicode 11)
and wires resize and theme changes.

- **Input** is raw bytes. Ctrl+C, Ctrl+D and Ctrl+Z are not special-cased. xterm.js
  emits `0x03`, `0x04` and `0x1a`, and the PTY line discipline turns them into
  SIGINT, EOF and SIGTSTP for the foreground job.
- **Output** is raw bytes. The native layer never parses ANSI.
- **Scrollback** is 10,000 lines, kept in memory only and never persisted.
- **Terminal-specific risks:** OSC 52 clipboard writes and clickable links are off,
  because the add-ons that provide them are not installed (`docs/security.md`
  §3.1).

## 7. Workspace, files and editor [built: Phase 2]

```
Open Folder ─▶ workspace_open ─▶ native folder picker ─▶ Workspace (cap-std handle on the root)
                                                          └▶ watcher (FSEvents) ─▶ Channel<WorkspaceEvent>
Explorer ─▶ workspace_list_dir(path)                       one level, when a folder is expanded
Editor   ─▶ workspace_read_file(path)                  ──▶ { text, version }
         ─▶ workspace_write_file(path, text, version)  ──▶ new version  |  conflict (nothing written)
Terminal ─▶ terminal_create                            ──▶ login shell in the workspace root
```

**The workspace** is one folder the user chose in the native picker (ADR 0009).
Opening another replaces it, after offering to save unsaved tabs. A page reload
closes it. Everything inside is addressed by *workspace path* (relative, `/`),
validated in `crates/workspace` and resolved through a `cap-std` handle that
cannot leave the root, symlinks included.

**The explorer** (`src/workspace/`) lists directories one level at a time, only
when expanded, so a large repository is never loaded whole. Watcher events reload
only the loaded directories they touch. Create and rename use an inline name
field. Delete asks, then moves the entry to the Trash. Every failure appears as a
notification.

**The editor** (`src/editor/`, ADR 0008) has one CodeMirror view and one editor
state per tab (text, undo history, selection).

| Situation | What happens |
| --- | --- |
| Text differs from what was last read or saved | The tab is dirty (●) |
| Saving | Sends the version the tab was based on. Writes are atomic and keep permissions. |
| The file on disk is no longer that version | The save is refused, with **Overwrite** or **Revert to Disk** offered |
| A clean tab changes on disk | It reloads, as a transaction that can be undone |
| A dirty tab changes on disk | It is only marked, and a banner offers the same two choices |
| The file is deleted | The tab stays open, marked. Saving recreates the file. |
| The app's own saves come back from the watcher | Recognised by version and ignored |

Binary files (a NUL byte, or bytes that are not UTF-8) and files over 32 MB are
not opened, and files over 2 MB open without syntax highlighting.

**The terminal** is unchanged from Phase 1 except for the directory it starts in.
`terminal_create` takes the root of the open workspace from native state; the
webview still cannot choose. Opening a workspace adds a terminal tab there, and
existing sessions are never moved.

**The workbench** (`src/workbench/`) implements every user action and coordinates
explorer, editor and terminals, so components stay presentational and behaviour is
tested without React.

| Shortcut | Command |
| --- | --- |
| ⌘O | Open Folder |
| ⌘P | Go to File (fuzzy search over files gathered on demand, respecting `.gitignore`) |
| ⇧⌘P | All commands |
| ⌘N | New File |
| ⌘S / ⌥⌘S | Save / Save All |
| ⌘W | Close Editor (asks if unsaved); in a terminal, Close Terminal Pane |
| ⌘B | Toggle Sidebar |
| ⇧⌘E / ⇧⌘F | Files / Search in the sidebar |
| ⌃R | Open Recent Folder |
| ⌃` / ⌃⇧` | Toggle Terminal / New Terminal |
| ⌘D / ⇧⌘D (in a terminal) | Split Terminal Right / Down |
| ⌘] / ⌘[ (in a terminal) | Next / Previous Terminal Pane |

In the explorer, the arrow keys move and expand, Enter opens, F2 renames and ⌘⌫
moves the selection to the Trash. Search inside a file is CodeMirror's (⌘F, ⌘G,
⌥⌘F). The macOS menu has no Close
Window item, so ⌘W reaches the app. The panels are resizable with the mouse or
the arrow keys, and their sizes are remembered locally.

**Quitting** asks first when it would lose something (Phase 3 below).

## 7a. Workspace sessions: recent folders, trust, search and splits [built: Phase 3]

```
launch ─▶ workspace_recent ─▶ workspace_open_recent(most recent, if it exists) ─▶ first terminal there
status bar "Untrusted" ─▶ workspace_set_trust(true) ─▶ native NSAlert ─▶ TrustStore (only if confirmed)
⇧⌘F ─▶ workspace_search(query, channel) ─▶ walk ─▶ N × (cap-std open ─▶ grep-searcher) ─▶ File{path, matches}… Done{summary}
⌘D ─▶ Terminals.split ─▶ new pane ─▶ TerminalView ─▶ terminal_create (a new PTY, in the workspace root)
Dock Quit / logout ─▶ applicationShouldTerminate: ─▶ must_ask? cancel + QuitRequested : quit
```

**Recent folders and trust** (ADR 0010). `crates/workspace/src/store.rs` keeps two
JSON files in the app data directory: the 15 most recently opened folders and the
folders the user trusted, as absolute paths and timestamps only. At launch the
most recent folder is reopened if it still exists. The explorer's empty state and
⌃R list the others; missing folders are shown as such and can be removed.
Reopening is limited to folders in the list and to paths that still lead to the
same folder. Trust is shown in the status bar, granted only through a native
dialog, removable at any time, and exact to one folder. It is recorded and
queryable (`Workspaces::is_trusted`), and since Phase 4 required for starting an
agent; removing it stops the folder's agents and forgets their approvals.

**Search** (`crates/workspace/src/search.rs`). A literal, optionally
case-sensitive search over the workspace. One thread walks the tree and up to
eight search files, because opening and reading many small files is dominated by
system calls (100,000 small files: about 2.6 s, first results within
milliseconds; ripgrep takes about 1.6 s on the same tree). It walks with the
same rules as quick open (`files::walker`: `.gitignore` and friends respected,
hidden files included, symlinks not followed, and `.git`, `node_modules`,
`target`, `dist`, `build` and other dependency or build directories skipped) and
reads each file through the workspace's `cap-std` handle with ripgrep's searcher,
which stops at the first NUL byte (binary files are skipped). Each file's matches
are streamed as they are found, with UTF-16 columns (the editor's unit) and a
preview shortened around the match; a summary follows. Limits: 5,000 matches, 200
per file, files up to 16 MB. A new search, or closing the workspace, cancels the
previous one. The frontend (`src/workspace/search.ts`) searches 250 ms after
typing pauses, shows only the latest search's results, and opens a result with
the match selected and scrolled into view (`EditorStore.select`).

**Split terminals** (`src/terminal/panes.ts`, `terminals.ts`). Each terminal tab
holds a binary tree of panes; each split is `right` (side by side) or `down`, with
a ratio the divider adjusts (mouse or arrow keys, 10–90 %). Every pane is its own
`TerminalView` and therefore its own native session: its own PTY, shell, process
group and working directory. New panes start in the workspace root, like new
tabs. Views are rendered in creation order and positioned from the tree, so
splitting or closing never moves an existing terminal in the DOM (which would
restart its renderer). Closing a pane gives its space to its sibling; closing a
tab's last pane closes the tab.

**Closing and quitting** ask first when something would be lost:

| Action | Asks when | Question |
| --- | --- | --- |
| Close a pane or tab | A program runs in its foreground (`terminal_is_busy`) | End the running program? |
| ⌘W on an editor tab, open another folder | The tab or any tab is unsaved | Save, Don't Save, Cancel |
| ⌘Q, close the window, Dock Quit, logout, shutdown | Any tab is unsaved, or any terminal runs a program | Save…, then Quit and end N running programs? |

The native side decides whether quitting may be immediate (`app::must_ask`:
unsaved changes reported by the frontend, or any session with a foreground job,
and a frontend subscribed to ask). If not, it sends `QuitRequested` and quits only
when the frontend calls `app_quit`. Dock Quit, logout and shutdown reach the same
check through an `applicationShouldTerminate:` method added to tao's application
delegate (ADR 0011); a logout is cancelled while the question is open. Force
Quit, `kill -9` and crashes cannot be intercepted: unsaved edits are lost then,
and the kernel still hangs up every terminal.

**Other state.** Panel sizes, visibility and the sidebar view are remembered in
`localStorage` (a convenience; defaults apply without it). Open editor tabs are
not restored across launches yet.

## 8. Agent architecture [built: Phases 4–5; adapters: Phases 6–8]

The runtime is described in full in `docs/agent-runtime.md` (ADR 0012), and
several agents at once, each in a Git worktree of its own, in
`docs/multi-agent.md` (ADR 0013).

Every target agent (Claude Code, OpenCode, Codex, Aider) is primarily an
interactive terminal program. So the agent runtime is built on the session
substrate, not beside it:

```
AgentRuntime = AgentDefinition (data) + launch in a Session (Phase 1)
             + per-agent config adapter (model: Phase 6; MCP: Phase 7; skills: Phase 8)
```

- **`AgentDefinition`** [built, `crates/core/src/agent.rs`] declares an id, name,
  `LaunchSpec` (program, args, env), requirements (for example "`claude` must be
  on PATH"), and capabilities: which model APIs the agent can use
  (`modelApis`) and which MCP transports it supports (`mcpTransports`).
- **The runtime** [built, `crates/agents`] detects installed agents on the
  user's login `PATH`, resolves the program to an absolute path, and starts it on
  a PTY session with the workspace as cwd, but only once the workspace is trusted
  and the user approved exactly that launch in exactly that workspace. It maps
  onto the session interface: start = spawn (`agent_start`), sendInput = write,
  resize, status = session state, output = channel, stop = hangup. Approvals are
  stored per workspace in the app data directory, never in the project.
- **Config adapters** [built: models Phase 6, MCP Phase 7, skills Phase 8,
  `crates/agents/src/adapter/`] translate "use provider P with model M (and MCP servers S, and skills K)" into
  what each agent actually reads: documented flags, environment variables or inline
  configuration. This is the one place agent-specific code is allowed, isolated
  per agent behind one trait (`AgentAdapter`), looked up by agent id. Claude Code
  and OpenCode have one; Codex has one for its model (OpenAI, through its
  Responses API).
- **Structured mode** [future, after Phase 4]. Some agents also offer machine
  interfaces: headless JSON streams, local servers, and the Agent Client Protocol
  (ACP). A second runtime kind can use these for richer UI (diff review, tool call
  approval) without changing the definition format.

The app never bypasses an agent's own safety model. For example, it does not add
"skip permissions" flags by default (`docs/security.md`).

## 9. Model architecture [built: Phase 6, docs/models.md]

- **`ModelProviderDefinition`** [built, `crates/core/src/model.rs`] declares
  endpoints, auth and known models. Each endpoint is a (`ProviderApi`, `baseUrl`)
  pair. The APIs are wire protocols: `anthropicMessages`,
  `openAiChatCompletions`, `openAiResponses` and `gemini`.
  Built-in providers [built, `crates/providers`]: Anthropic, OpenAI, Google,
  OpenRouter (a gateway) and Ollama (local). Models come from the definition,
  from Ollama, or from ids the user adds; no capability is claimed unless
  verified.
- **Compatibility is decided by the agent's adapter**, using the wire APIs: an
  adapter picks the provider endpoint whose `ProviderApi` the agent speaks and
  knows how the agent authenticates there. Ollama serves both OpenAI-compatible
  and Anthropic-compatible endpoints, so the same local model can serve Claude
  Code and OpenCode (ADR 0016).
- **The app does not proxy model traffic.** Agents talk to providers directly. The
  app's job is to store credentials (Keychain), list the models it knows, and
  configure agents at launch. It makes no request to hosted providers; connection
  tests and remote model lists are deferred. A local gateway that translates
  between protocols is a possible later addition. It would get its own ADR,
  because it would put the app on the data path of every prompt.
- **Local models** [built: Ollama detection and its model list, on request only;
  planned: pulls, other local servers]: "runs on this machine" is shown as such,
  since it means code never leaves the device.
- **Model selection is per agent session**, kept with the session and its worktree.
- **Environment precedence** (ADR 0015): an app-configured session replaces every
  variable the adapter controls; otherwise the agent's own configuration is
  untouched. Provider and endpoint are part of the approval; the model is not.
- **Credentials** are referenced by `SecretName` only (§11).

## 10. MCP architecture [built: Phase 7, docs/mcp.md, ADR 0017]

- **The registry** [built, `crates/mcp`] holds the servers the user added
  (`McpServer`, `crates/core/src/mcp.rs`): stdio (a command and structured
  arguments, never a shell) or Streamable HTTP (a URL); variables by name, with a
  secret or inherited source; enabled; scope global, workspace or session.
  Secrets are in the Keychain. The Phase 0 `McpServerDefinition` is unchanged:
  the Phase 8 catalog lists the registry's servers and distributes no definitions
  (ADR 0018).
- **The agent is the MCP client.** For each agent session, the app selects the
  enabled servers the session gets and that the agent's adapter and
  `mcpTransports` support; the adapter tells the agent where they are, for that
  session only (`--mcp-config` for Claude Code, `OPENCODE_CONFIG_CONTENT` for
  OpenCode).
- **Stdio servers are the session's processes, started by the app.** The agent
  runs a bridge to a private socket; when the session's agent connects, the app
  starts the approved program with a controlled environment, and stops it with
  the agent. HTTP servers are contacted by the agent only.
- **Approval** pins, per workspace, exactly what runs: executable, arguments, URL,
  variables by name and source.
- **The app is not an MCP client** and has no inspection or test action: that
  would start servers or contact URLs outside a session.
- **MCP gateway** [deferred, ADR 0017]. Routing agent↔server traffic through the
  app would enable central policy and audit, at the cost of latency and
  complexity.
- Remote-server authentication is the agent's own (OAuth); app-held header tokens
  are not modelled yet.

## 11. Secrets [built: Phase 6, ADR 0014]

Definitions can reference secrets only by `SecretName` (for example `EnvValue::Secret`
or `ProviderAuth::ApiKey`). They can never contain values. The native secret store
(macOS Keychain, `crates/secrets`) is read when a specific agent session's agent
starts (once per app run: the value is then kept in the app's memory until the app
quits or it is replaced or removed), and the value is placed only in that
process's environment. Secret
values never cross IPC to the webview (it can save and delete a key, and see
whether one exists), never appear in logs or error messages (`SecretValue` and every
environment-carrying type print names only), and are never stored in files the app
writes. `EnvValue::Secret` in an agent definition is still refused: provider keys
reach agents only through adapters. Validation rejects URLs with embedded credentials and API-key traffic
over plain HTTP to non-loopback hosts. Both rules are built and tested.

## 12. Catalog architecture [built: Phase 8, docs/catalog.md, ADR 0018]

The catalog is discovery and orchestration, not execution.

- **Items** (`CatalogItem`, `crates/core/src/catalog.rs`) are agents, model
  providers and models, MCP servers and skills, with stable ids
  (`agent.claude-code`, `model.anthropic.claude-sonnet-5`, `mcp.github`,
  `skill.tests-first`), a status (installed, configured, available, unavailable,
  unsupported), requirements, capabilities and typed details.
- **One source of truth.** `x8ai-catalog` assembles items from what each owning
  system reports: the agent runtime (`x8ai_agents::status`), the provider
  registry (`x8ai_providers::status`), the MCP registry and the skill registry.
  Its own metadata (`builtin.json`) is presentation only: publisher when known,
  tags, a version. Its fields are closed, so it cannot configure anything.
- **Actions are requests** to the owning system's commands (open Agents, set up
  in Models, enable or configure an MCP server, add a skill), or fill in the next
  launch's choices. Launching is unchanged: one approval dialog, then the session.
- **Sources:** built in, local (Ollama's models), user-defined (servers, model
  ids, skills). Remote is a reserved value: refused, and nothing fetches.
- **Future:** signed remote metadata, verified packages, and installation through
  the owning system behind its approval. The seams are `MetadataSource`,
  `SignedMetadata` and `Verifier`, none implemented remotely.
- **Skills** (`x8ai-skills`) are instructions for a session, delivered by the
  agent's adapter (Claude Code: `--append-system-prompt`), recorded by id,
  version and fingerprint, and never silently upgraded.

## 13. Security boundaries

Summarized here and detailed in `docs/security.md`:

| Boundary | Enforced by | Status |
| --- | --- | --- |
| Webview → native | Tauri capabilities (explicit per-command grants), CSP with strict `script-src`, no shell/fs plugins | [built] |
| Native → child process | The webview can start only the login shell, and never supplies a program. Process-group teardown. | [built: terminal] |
| Native → agent process | Built-in definition, resolved absolute path, no shell, workspace trust + per-workspace approval checked natively on every start | [built: Phase 4] |
| Native → agent process (models) | A provider key only for the session it was chosen for; provider and endpoint pinned in the approval; the shell's provider variables replaced, not mixed | [built: Phase 6] |
| Native → MCP server process | Per-workspace approval of exactly what runs; a started-on-connection process of the session, with a controlled environment (no provider keys), killed with the session | [built: Phase 7] |
| Workspace boundary for the app's own file operations | Folder chosen in the native picker (or reopened from the recent list of such folders); every operation, search included, through a `cap-std` handle on the root; validated workspace paths | [built: Phases 2–3] |
| Workspace trust | Granted only in a native dialog, exact to one folder, stored outside it (ADR 0010); required to start an agent | [built: Phase 3; enforced for agents: Phase 4] |
| Catalog → owning systems | A pure crate with no process, network, Keychain, trust or approval access; actions are requests to the owning system's commands; metadata cannot configure anything | [built: Phase 8] |
| Skills → agent | Text only, no secrets (validated), for one session through the agent's documented flag; a session runs only with the exact skills it recorded | [built: Phase 8] |
| Integration trust (remote catalog) | Signed metadata, verified packages, installation through the owning system's approval | [future] |
| Inside a child process | **Not enforced.** Children run with user privileges. OS-level sandboxing is future work. | — |

## 14. Extension strategy

1. **New agent, provider or MCP server:** add a definition (data). No code changes
   if its protocols are already known.
2. **New agent with unusual configuration:** add a config adapter in the agents
   crate, implementing one trait.
3. **New wire protocol or transport:** add an enum variant in `x8ai-core`. Tests and
   the generated TypeScript make every consumer handle it.
4. **No in-process plugins.** Third-party code is not loaded into the app process or
   the webview. Integrations run as separate processes. If in-process
   extensibility is ever needed, a sandboxed runtime such as WebAssembly would be
   evaluated in its own ADR.

## 15. Open decisions

| Decision | Phase | Notes |
| --- | --- | --- |
| MCP gateway | 7+ | Needed for central policy and audit? |
| Remote catalog format and signing | Future | Signed static index vs service; key management (ADR 0018). |
