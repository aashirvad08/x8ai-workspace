# Security

x8ai Workspace will run shells, AI agents, AI-generated commands, MCP servers and
other local processes on the user's machine. That is the product. The goal is not
to prevent execution. The goal is to make sure that everything that executes was
started deliberately, by the right party, with no more access to secrets than it
needs, and that the user can see what is happening.

This document records the threat model, the risks by area, and what is actually
enforced today. Anything marked **not yet enforced** is a design commitment, not a
protection. Do not rely on it.

---

## 1. What is enforced today (Phases 0–2)

These protections exist and are verified:

| Control | Where | Verified by |
| --- | --- | --- |
| Every native command needs an explicit grant to a window. Tauri rejects ungranted calls before the command's code runs. | `src-tauri/build.rs`, `src-tauri/capabilities/` | Manual: revoking the grant yields `Command get_app_info not allowed by ACL` in the UI |
| The webview's commands are `get_app_info`, `app_*` (quit guard), `terminal_*` and `workspace_*`, each granted explicitly. No shell, fs, HTTP or opener plugins are installed. `tauri-plugin-dialog` is used only from Rust, and none of its webview commands are granted. | `src-tauri/Cargo.toml`, `capabilities/main-window.json` | Code review, `tauri-build` ACL output |
| **The webview cannot choose what a terminal runs, or where.** `terminal_create` always starts the user's login shell, in the open workspace's root or the home directory, and accepts only a size. | `src-tauri/src/terminal.rs` | Code review; manual `pwd` |
| **The webview cannot name a folder to open.** A workspace is whatever the user picks in the native folder picker. | `src-tauri/src/workspace.rs` | Code review |
| **File operations cannot leave the workspace.** Workspace paths are validated (no absolute paths, `..`, `.`, empty segments or NUL), then resolved through a `cap-std` handle on the root, which refuses symlink escapes. | `crates/workspace` | Integration tests (`rejects_paths_that_leave_the_workspace`, `symlinks_cannot_reach_outside`) |
| **Saves never silently overwrite an external change,** and never leave a half-written file: version-checked, atomic replace, permissions kept | `crates/workspace/src/workspace.rs` | Tests; manual conflict check |
| **Delete is recoverable.** Entries move to the Trash (NSFileManager, no `osascript`), after a confirmation dialog whose default is Cancel. | `crates/workspace`, `src/workbench/workbench.ts` | Manual |
| Unsaved changes are not lost on ⌘W, on opening another folder, or on ⌘Q and window close. Dock Quit and logout are not intercepted yet. | `src/workbench/workbench.ts`, `src-tauri/src/app.rs` | Unit tests; manual ⌘W |
| Terminal command arguments are validated in Rust: sizes (1–4096 × 1–2048), session ids (`notFound` otherwise), and raw input only to an existing session | `crates/core/src/terminal.rs`, `crates/pty` | Unit and integration tests |
| No terminal content is persisted. Scrollback (10,000 lines) exists only in webview memory. Native output buffering is bounded by flow control (512 KiB per session). | `src/terminal/TerminalView.tsx`, `crates/pty/src/session.rs` | Tests (`output_pauses_until_acknowledged`) |
| OSC 52 clipboard writes and clickable links are off: the xterm.js add-ons that implement them are not installed | `package.json` | Review |
| Terminal processes do not outlive their session or the app. Close, reload, quit and crash all hang up the terminal, and a shell ignoring SIGHUP gets SIGKILL after 2 s. | `crates/pty`, `src-tauri/src/lib.rs` | Tests; manual `ps` checks after Cmd+Q, SIGTERM and Ctrl+D |
| Content Security Policy: `script-src 'self'` with Tauri's nonces and hashes (no inline or remote scripts, no `eval`), no plugins or frames, IPC-only `connect-src`. Inline styles are allowed for xterm.js (ADR 0007). | `src-tauri/tauri.conf.json` | Production build runs under it |
| Only `src/native/` can talk to Tauri, only `src/terminal/` uses xterm.js, only `src/editor/` uses CodeMirror, and nothing but `main.tsx` depends on the UI layer | `src/architecture.test.ts` | CI |
| Integration definitions reference secrets by name only. There is no field that can hold a secret value. | `crates/core/src/{launch,model}.rs` | Type design, tests |
| Endpoint URLs must be `http(s)`, must not embed credentials, and must use HTTPS for API keys or MCP traffic to non-loopback hosts | `crates/core/src/definition.rs` | Unit tests |
| Integration ids are restricted to `[a-z0-9-]`, so they cannot carry path or shell metacharacters. Unknown definition fields are rejected. | `crates/core/src/id.rs`, `deny_unknown_fields` | Unit and integration tests |
| Vite dev server bound to `localhost` only | `vite.config.ts` | Config |
| No credentials in the repository. `.env*` is git-ignored. | `.gitignore` | Review |

Everything else in this document is **not yet enforced**.

## 2. Threat model

**Assets.** The user's source code, credentials (API keys, tokens, SSH keys, cloud
credentials, all reachable from the user account), the machine itself, and the
integrity of what the user believes they approved.

**Adversaries and failure sources**

1. **Untrusted content in a workspace.** A cloned repository can carry prompt
   injections aimed at agents, malicious agent or MCP configuration files, git
   hooks and build scripts.
2. **A compromised or malicious integration.** An MCP server, agent, npm package,
   Docker image or catalog entry that behaves maliciously, or changes behaviour
   after approval ("rug pull").
3. **A confused or manipulated AI agent.** It runs a destructive or exfiltrating
   command it believes is helpful.
4. **Webview compromise.** XSS through rendered content (Markdown, tool output,
   file names, error messages) that then drives the IPC surface.
5. **Network attackers** against provider or MCP traffic, and against update or
   catalog downloads.

**Key trust fact.** Every child process runs with the user's full privileges. Once
running, a shell, agent or MCP server can read `~/.ssh`, reach the network, and
spawn anything. The app does not sandbox child processes today, and it does not
claim to. Controls therefore focus on what the app does decide: whether to start
a process, with which exact command line, in which directory, and with which
secrets.

## 3. Risks by area

### 3.1 Arbitrary shell execution

A terminal is arbitrary execution by design, and that is fine when the user types
the command. The risk is execution the user did not initiate.

- **Webview compromise equals remote code execution** once terminal sessions exist.
  Any code running in the webview can call `terminal_write` and type into a live
  shell. Mitigations:
  - Treat the webview as untrusted (ADR 0005). Keep the strict CSP. Load no remote
    content in the main webview. Never use `dangerouslySetInnerHTML` or
    unsanitized Markdown rendering. Render terminal output only through xterm.js,
    never as HTML.
  - Keep the IPC surface narrow. The webview can write bytes to sessions that
    already exist. It cannot choose the program a session runs, because the shell
    comes from native configuration, and it cannot spawn arbitrary processes.
  - **Enforced (Phase 1):** session commands validate ids and sizes, and session
    creation takes no program path from the webview.
- **Escape sequences are an attack surface.** Output from a malicious `cat`ed file
  can try to write the clipboard (OSC 52), spoof links (OSC 8) or set the window
  title. **Enforced (Phase 1):** OSC 52 and link handling are not installed, so
  neither can do anything. The window title is not bound to terminal titles. If
  links are ever added, they must show their real target and open only through a
  native, allowlisted opener.
- **No shell interpolation anywhere.** The native layer starts programs from
  `program + args` (`LaunchSpec`) and never builds a `sh -c` string from data.

### 3.2 AI-generated commands

Agents such as Claude Code, Codex and OpenCode run commands they generate. Each has
its own approval model. That model is the primary control, and the app must not
weaken it.

- The app never launches an agent with approval-bypass or "yolo" flags by default.
  If a user opts in, the choice is per workspace, visible in the UI, and remembered
  explicitly. **Not yet enforced (Phase 4).**
- **Prompt injection** from repository content, web pages or MCP tool results can
  steer an agent. The app cannot solve this. It reduces the impact by giving each
  agent session only the secrets it needs, not placing app-held secrets in the
  environment by default, and supporting disposable git worktrees (Phase 8), so
  changes can be reviewed before they touch the main checkout.
- **OS-level sandboxing** of agent sessions (macOS Seatbelt profiles, containers or
  VMs) is an explicit research item for Phase 12. Some agents already ship their
  own sandbox. The app should surface and prefer those rather than stack a
  second, incompatible one.

### 3.3 MCP tools

- **A stdio MCP server is arbitrary local code** with user privileges. Starting one
  is equivalent to running an installer. **Phase 7:** show the exact command line,
  the resolved executable path and the secrets passed before first start, then
  require approval.
- **Unpinned versions** (`npx pkg@latest`, `docker … :latest`) let a server change
  under the user. Built-in and catalog definitions must pin versions (the test
  fixtures do). Pin by digest where the ecosystem allows it.
- **Tool poisoning.** Malicious tool descriptions can instruct the agent. **Phase 7:**
  show tool lists and descriptions at enable time, and re-prompt when they change.
- **Rug pulls.** Record a content hash of each approved definition and require
  re-approval when it changes. **Phase 10.**
- **Remote MCP servers** receive workspace content through tool calls. HTTPS is
  required for non-loopback hosts (enforced in definition validation). Auth
  follows the MCP OAuth specification, and tokens are held natively and scoped to
  one server. **Phase 7.**
- **Project-scoped MCP configuration in a repository** (for example files that
  agents read automatically) must never be started by the app without workspace
  trust (§3.8).

### 3.4 Secrets and API keys

- **Storage:** macOS Keychain via the native secret store. No plaintext config
  files, and not `localStorage`. **Phase 5.**
- **Reference, don't embed.** Definitions use `SecretName`, and the type system
  makes embedding a value impossible (built). Validation rejects URLs with
  embedded credentials (built).
- **Delivery:** secrets are resolved at launch and placed only in the environment
  of the specific child that needs them. They are never set on the app process,
  so shells and other children cannot inherit them. **Phase 4/5.**
- **Never in the webview.** No command returns a secret value. The UI can only set,
  replace or delete a secret and see whether one exists. **Phase 5.**
- **Never in logs, errors or crash reports.** `CommandError.message` must not include
  secret values or environment dumps (documented on the type). Redaction is added
  when logging lands.
- **Residual risk:** once a secret is in an agent's environment, the agent (and
  anything it runs) can read and exfiltrate it. Prefer agents' native login flows,
  where tokens stay in the agent's own store, over API keys the app injects.
- **Vite:** only `VITE_*` variables are exposed to the frontend bundle. Never put
  secrets in them.

### 3.5 Filesystem access

- **Enforced (Phase 2):** the app's own file operations (file tree, editor open
  and save, create, rename, delete, quick-open listing) are native commands
  confined to the workspace root, which the user chose in the native picker. Paths
  are validated, then resolved through a `cap-std` directory handle, which rejects
  `..` and symlink escapes. It also closes most check-then-use races, because
  resolution happens beneath the handle rather than by string prefix (ADR 0009).
- **Residual races (accepted):** between the save's version check and its rename,
  between a rename's existence check and the rename, and between the delete's
  check and the Trash call, which takes an absolute path. They are brief, and they
  involve the user's own actions.
- The Tauri `fs` plugin is not used. The webview never gets general file access.
- **Honest limit:** the workspace boundary constrains the app's commands only.
  Shells, agents and MCP servers are not confined by it (§2). A compromised webview
  can read and change anything inside the chosen workspace, which is what
  choosing it grants.
- Nothing is uploaded or indexed remotely by the app itself. Quick open walks the
  tree on demand and stores nothing. Warning about sensitive files (`.env`, keys)
  is planned, not built.

### 3.6 External processes

- **PATH hijacking.** A GUI app's `PATH` differs from the user's shell. Programs are
  resolved to absolute paths through the user's login environment, and the
  resolved path is shown at approval. A binary appearing earlier on `PATH` later
  changes the resolved path, which triggers re-approval. **Phase 4.**
- **Orphans and runaway processes.** Every terminal process is a session leader
  tracked in a registry. Close, reload and quit send SIGHUP to it and its
  foreground job, then SIGKILL to its process group after a grace period. Jobs the
  user detached on purpose (`nohup`, `disown`) survive, as in any terminal.
  **Enforced (Phase 1).**
- **Environment leakage.** Terminal sessions inherit the app's environment plus
  `TERM`, `COLORTERM`, `TERM_PROGRAM` and, when no locale is set, `LANG`. The app
  holds no secrets yet, so none can leak. When secrets arrive (Phase 5) they are
  never placed in the app's own environment. Under `pnpm tauri dev`, sessions also
  inherit the dev server's environment. **Phase 1 (terminal), Phase 4/5 (agents).**
- **Resource exhaustion.** Output is batched with bounded buffers and backpressure:
  a flood blocks the producer rather than growing memory. **Enforced (Phase 1).**
  Showing the number of sessions arrives with tabs.

### 3.7 Malicious integrations and the catalog

- **Supply chain:** typosquatted packages, compromised npm or PyPI releases with
  install scripts, and a compromised catalog index. **Phase 10:** a signed catalog
  index, pinned versions and checksums, provenance display, reviewed built-ins,
  and user-defined entries labelled untrusted.
- **Installation executes code.** "Install" shows the exact commands (for example
  `npm i -g …`) and runs them only after approval, in a visible terminal session,
  never silently.
- **No in-process plugins.** Third-party code never loads into the app process or
  the webview (architecture §14).

### 3.8 Workspace trust

Opening a folder must not execute anything from it. Before a workspace is trusted,
the app does not auto-start project-defined MCP servers, agent configurations,
tasks or hooks. Trust is granted per folder and can be revoked. **Phase 3**, with
enforcement points added in Phases 4 and 7. Today (Phase 2), opening a folder only
lists and reads files; nothing in it is executed.

### 3.9 The webview and IPC

- The webview is untrusted by design (ADR 0005). Command arguments are validated
  in Rust as untrusted input. Return values contain no secrets.
- Capabilities stay per window and minimal. A future window that renders
  untrusted content, such as a Markdown preview or browser pane, gets its own
  capability with no commands.
- `script-src` stays strict. Inline styles are allowed because xterm.js generates
  its styles at runtime, and `freezePrototype` is off because xterm.js cannot run
  with a frozen `Object.prototype`. See ADR 0007 for the trade-off and when to
  revisit it.
- The devtools inspector is available in debug builds only. Release builds do not
  enable the `devtools` feature.

### 3.10 Distribution and updates

**Phase 12:** Developer ID signing, notarization, the hardened runtime with a minimal
entitlement set, and signed updates (Tauri updater with signature verification,
HTTPS only). The Mac App Store sandbox is incompatible with a tool that must
launch arbitrary developer tools. This trade-off is accepted and documented, and
the controls above compensate where they can.

### 3.11 Privacy

The app sends nothing to any server of its own. There is no telemetry. Model and
MCP traffic goes directly from agents to the providers the user configured. If
crash reporting is ever added, it is opt-in and scrubbed of paths, environment
and content.

## 4. Rules for contributors

1. Every new native command must be granted explicitly and validate its arguments.
   Review capability diffs as security diffs.
2. Never spawn through a shell string. Use `program + args`.
3. Never pass a secret value through IPC, a log line, an error message or a file
   the app writes.
4. Never render untrusted content as HTML in the main webview.
5. Never auto-execute anything from a workspace the user has not trusted.
6. Never add approval-bypass flags to an agent launch by default.
7. Pin versions in any built-in definition.
8. Do not describe a control as enforced until it is, and a test or check proves it.
