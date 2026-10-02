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

## 1. What is enforced today (Phases 0–8)

These protections exist and are verified:

| Control | Where | Verified by |
| --- | --- | --- |
| Every native command needs an explicit grant to a window. Tauri rejects ungranted calls before the command's code runs. | `src-tauri/build.rs`, `src-tauri/capabilities/` | Manual: revoking the grant yields `Command get_app_info not allowed by ACL` in the UI |
| The webview's commands are `get_app_info`, `app_*` (quit guard, startup warnings), `terminal_*`, `workspace_*` (files, recent folders, trust, search), `agent_*`, `provider_*` (keys, model ids), `mcp_*` (MCP servers and their secrets), `skill_*` (the user's skills) and `catalog_list`, each granted explicitly. No shell, fs, HTTP or opener plugins are installed. `tauri-plugin-dialog` is used only from Rust, and none of its webview commands are granted. | `src-tauri/Cargo.toml`, `capabilities/main-window.json` | Code review, `tauri-build` ACL output; `src/architecture.test.ts` checks that the grants are exactly the commands the client calls |
| **The webview cannot choose what a terminal runs, or where.** `terminal_create` always starts the user's login shell, in the open workspace's root or the home directory, and accepts only a size. | `src-tauri/src/terminal.rs` | Code review; manual `pwd` |
| **The webview cannot name a new folder to open.** A workspace is whatever the user picks in the native folder picker, or a folder from the recent list, which only ever holds folders picked that way. The Welcome screen's `/cd <folder>` can only say where the picker starts; the user still chooses the folder in it. Reopening also requires the path to still lead to the same folder, so replacing it with a symlink does not redirect it. | `src-tauri/src/workspace.rs`, `crates/workspace` | Code review; test `reopening_refuses_a_path_that_now_leads_elsewhere` |
| **Only the user can trust a folder.** Trust is granted only in a native confirmation dialog the webview cannot answer, applies to exactly one folder (not its parent or subfolders), and is stored outside the folder, so a repository cannot declare itself trusted (ADR 0010). | `src-tauri/src/workspace.rs`, `crates/workspace/src/store.rs` | Store tests (`trust_is_explicit_exact_and_persistent`); code review |
| **What the app remembers is minimal.** Recent and trusted folders are stored as absolute paths and timestamps only, in files readable only by the user (0600, directory 0700), replaced atomically. A damaged file is set aside, never silently deleted. No file names or contents are stored. | `crates/workspace/src/store.rs` | Store tests |
| **Search stays inside the workspace.** It reads files only through the workspace's `cap-std` handle, never follows symlinks, skips `.git`, dependency and build directories, ignored files and binary files, caps results (5,000 matches, 200 per file, 16 MB per file) and is cancelled by the next search or by closing the workspace. | `crates/workspace/src/search.rs`, `files.rs` | Search tests (`never_leaves_the_workspace_through_symlinks`, `skips_generated_ignored_and_binary_files`, …) |
| **File operations cannot leave the workspace.** Workspace paths are validated (no absolute paths, `..`, `.`, empty segments or NUL), then resolved through a `cap-std` handle on the root, which refuses symlink escapes. | `crates/workspace` | Integration tests (`rejects_paths_that_leave_the_workspace`, `symlinks_cannot_reach_outside`) |
| **Saves never silently overwrite an external change,** and never leave a half-written file: version-checked, atomic replace, permissions kept | `crates/workspace/src/workspace.rs` | Tests; manual conflict check |
| **Delete is recoverable.** Entries move to the Trash (NSFileManager, no `osascript`), after a confirmation dialog whose default is Cancel. | `crates/workspace`, `src/workbench/workbench.ts` | Manual |
| Unsaved changes are not lost on ⌘W, on opening another folder, or on quitting: ⌘Q, closing the window, and on macOS Quit from the Dock, logout and shutdown (ADR 0011). Force Quit, `kill -9`, crashes and power loss cannot be intercepted by any app. | `src/workbench/workbench.ts`, `src-tauri/src/app.rs`, `src-tauri/src/macos.rs` | Unit tests; manual |
| **No agent runs in an untrusted folder, or without the user's approval for that folder.** `agent_start` takes only an agent id and a size. The program comes from a built-in definition, resolved to an absolute path on the user's login `PATH`; it runs in the open workspace, never through a shell. Trust and the approval (exact agent, executable, arguments and folder) are checked natively on every start, and a start without them does not type-check (`Authorized`). | `crates/agents`, `src-tauri/src/agents.rs` | Tests (`an_untrusted_workspace_blocks_the_launch`, `a_trusted_workspace_without_approval_blocks_the_launch`, `an_approval_for_one_workspace_does_not_cover_another`, `a_different_executable_on_the_path_needs_a_new_approval`) |
| **Only the user can approve an agent, and only per folder.** Approval is granted in a native dialog showing the folder and the exact command line; stored in the app data directory (0600), never in the project; revocable; removed with the folder's trust. | `crates/workspace/src/store.rs`, `src-tauri/src/agents.rs` | Store tests (`a_project_cannot_approve_agents_for_itself`, …) |
| **Agents stop when their permission ends:** when their terminal closes, another folder opens, the folder loses trust, or the app quits, with a question first while they run. | `crates/agents/src/runtime.rs`, `src/workbench/workbench.ts` | Tests (`closing_the_agents_terminal_ends_the_agent_and_its_children`, `quitting_the_app_ends_every_agent`, `opening_another_workspace_stops_…`, `removing_trust_can_stop_…`) |
| **Agents work in worktrees of their own, and the user's working tree is not touched.** In a Git repository each agent session gets a linked worktree on a new branch under the app-controlled `~/.x8ai/worktrees` (0700, no symlinks), named natively from the agent id and a generated token. The webview passes only ids; branch names and revisions are validated. Removal needs the user's confirmation and never deletes committed work. Non-Git folders take one agent at a time, stated as not isolated. | `crates/agents/src/isolation.rs`, `crates/git` | Tests (`multi_agent.rs`: separate worktrees, primary tree unchanged, paths in scope, symlinked directory refused, crafted metadata ignored, removal rules; `git.rs`: hooks do not run, `GIT_*` ignored, foreign revisions refused) |
| **The app's own Git calls run no repository code.** Hooks and fsmonitor are disabled, inherited `GIT_*` variables removed, no prompts, a timeout. | `crates/git` | Test (`the_apps_git_runs_no_repository_hooks_and_ignores_inherited_git_variables`) |
| **Built-in agents never launch with approval-bypass flags or secrets.** | `crates/agents/src/builtin.json` | Test (`no_builtin_agent_bypasses_its_own_approvals_or_needs_a_secret`) |
| **Provider keys live only in the macOS Keychain, and the webview can never read one.** The webview sends a key once, to save it; no command returns one; it learns only whether one is saved. No file the app writes holds a key (ADR 0014). | `crates/secrets`, `src-tauri/src/providers.rs` | Tests (`what_the_webview_receives_never_holds_a_key`, `the_key_reaches_the_agent_and_is_written_nowhere` scans every file, client test: no read method); live test with the real Keychain |
| **A key reaches only the agent session it was chosen for.** It is read natively when that session's agent starts (approving and creating the session only check that one is saved) and placed only in that process's environment; never in the app's own environment, so shells cannot inherit it. After the first read it is kept in the app's memory until the app quits or the key is replaced or removed, so macOS asks for the Keychain password at most once per app run. A removed key stops the next run. | `crates/agents/src/adapter/`, `src-tauri/src/agents.rs`, `crates/secrets` | Tests (a shell's environment inspected; `a_provider_that_needs_a_key_refuses_to_start_without_one`, `a_session_reads_its_key_only_to_start_and_once_per_app_run`, `a_kept_key_is_read_from_the_store_once_and_never_after_it_is_removed`); live test |
| **Nothing prints a key.** `SecretValue` has no `Display` or `Serialize` and a redacted `Debug`; `LaunchPlan`, adapter `Configuration` and the PTY `Environment` print variable names only; errors never echo input. | `crates/secrets`, `crates/agents`, `crates/pty` | Tests (`a_secret_is_never_printed`, `a_credential_never_appears_in_debug_output_or_errors`, `an_environment_prints_names_but_never_values`); review of every log line |
| **The shell cannot redirect an app-configured session, and the two are never mixed.** Every variable the agent's adapter controls is removed before the adapter's are set; Claude Code is told its provider is host-managed, so its settings files cannot change it; OpenCode's endpoint is pinned inline (ADR 0015). | `crates/agents/src/adapter/` | Tests (`claude_code_with_anthropic_uses_the_apps_key_and_nothing_from_the_shell`, …); live test with the real Claude Code |
| **The destination is approved.** An approval pins the provider and endpoint too; another provider or endpoint asks again; a model or key change does not. Endpoints come only from built-in definitions. Model ids are checked and can never become options. | `crates/workspace/src/store.rs`, `crates/core/src/model.rs` | Tests (`a_new_provider_or_endpoint_needs_approval_and_a_new_model_does_not`, `a_model_id_can_never_become_a_command_line_option`, …) |
| **An MCP server runs only for an agent session, in a trusted folder, once approved there exactly as it runs.** Approval pins the resolved executable, every argument, the URL, and the variables by name and source; any change asks again. Nothing starts at startup or because a server exists; a stdio server starts when the session's agent connects, and only a process of that agent may connect. | `crates/mcp`, `src-tauri/src/agents.rs` | Tests (`a_server_runs_only_in_a_trusted_workspace_once_approved_there`, `a_changed_command_arguments_endpoint_or_variables_need_a_new_approval`, `a_server_starts_only_when_the_sessions_agent_connects`, `a_connection_from_outside_the_session_starts_nothing`, `only_exactly_what_was_approved_can_be_started`); live test with the real Claude Code |
| **MCP servers are never run through a shell, and get only their own variables.** A command is one program (absolute or on the login `PATH`, never relative, never a shell); arguments are structured. The environment is a base (`PATH`, `HOME`, locale…), the server's listed inherited variables and its Keychain secrets: no provider key, no other server's secret. | `crates/core/src/mcp.rs`, `crates/mcp/src/environment.rs` | Tests (`a_stdio_command_is_one_program_never_a_command_line`, `a_servers_environment_is_the_base_its_variables_and_nothing_else`); live test (decoy provider keys absent from the server) |
| **MCP server processes do not outlive their session.** Startup timeout, bounded restarts and error output (redacted); process group killed when the agent ends, the folder loses trust, the page reloads or the app quits; stale sockets swept at startup. | `crates/mcp/src/runtime.rs` | Tests (`stopping_the_session_ends_the_server_and_everything_it_started`, `a_crash_is_reported_redacted_and_restarts_are_bounded`, `a_server_that_never_answers_is_stopped_after_the_startup_timeout`, `sessions_and_servers_are_isolated_and_quitting_leaves_nothing_running`) |
| **The catalog is data, and cannot act.** `x8ai-catalog` depends only on the contracts: no PTY, agent or MCP runtime, Keychain, trust or approval store, HTTP client or Tauri. Its source has no process, socket or file-writing API. `catalog_list` gathers the statuses each system already reports, starts no server and never probes Ollama. Its metadata has closed fields (no command, URL or setting), and metadata claiming a remote source is refused (ADR 0018). | `crates/catalog`, `src-tauri/src/catalog.rs` | Tests (`the_catalog_cannot_run_install_fetch_unlock_trust_or_approve_anything`, `listing_the_catalog_starts_probes_unlocks_and_approves_nothing`, `damaged_or_unsafe_metadata_is_refused`, `ollama_stays_unchecked_until_the_models_view_checks_it`, workbench: opening the catalog lists and probes nothing); live test |
| **Nothing is "installed" because metadata says so.** An agent is installed only when the runtime finds its program; metadata matching nothing the app has is not shown. The app installs nothing. | `crates/catalog/src/assemble.rs` | Tests (`an_agent_is_installed_when_the_runtime_finds_it_and_only_then`, `metadata_for_something_the_app_does_not_have_is_not_shown`) |
| **Skills are text for one session, and hold no secret.** A skill has instructions, suggested tools (never granted), a source and a scope; nothing that could change a provider, server, agent, trust or approval. Validation refuses control characters and anything that looks like a key. User skills are in `skills.json` (0600), apart from provider and MCP data. Claude Code gets them through `--append-system-prompt` for that session; OpenCode and Codex are refused. | `crates/core/src/skill.rs`, `crates/skills`, `crates/agents/src/adapter/` | Tests (`a_skill_can_never_hold_a_secret_and_the_file_holds_none`, `a_damaged_or_forged_file_loads_nothing_it_should_not`, `claude_code_gets_the_sessions_skills_through_append_system_prompt`, `agents_that_cannot_take_skills_are_refused_with_the_reason`) |
| **A session never runs with something other than what it recorded.** Its agent, model, MCP servers and skills (id, version, fingerprint) are recorded, in worktree metadata outside the worktree too. A removed or changed skill stops it with the reason; nothing is silently substituted or upgraded. | `crates/agents/src/runtime.rs`, `crates/skills/src/lib.rs`, `src-tauri/src/agents.rs` | Tests (`a_session_runs_only_with_the_skills_it_recorded`, `a_session_runs_only_with_exactly_the_skills_it_recorded`, `a_worktree_remembers_its_sessions_skills_by_reference_only`, `a_session_says_whether_each_skill_is_still_the_one_it_recorded`) |
| **Context handed between sessions is what the user sees and sends.** The composer (`/get`, `/give`) builds it from Git's view of a session's changes and the text its terminal shows; it never reads an agent's own transcripts or files, nor any key or secret of the app's. The exact text is shown, editable, with a warning that it all goes to the receiving agent and its provider. It is pasted only as a bracketed paste, with escape characters removed (nothing can end the paste early) and no trailing newline, so nothing runs and nothing is submitted until the user presses Enter; a program that has not turned bracketed paste on gets nothing. | `src/agents/context.ts`, `src/terminal/terminals.ts`, `src/workbench/workbench.ts` | Tests (`can be pasted without ending a bracketed paste early or pressing Enter`, workbench: `waits for the agent to take a paste, and sends nothing if it never does`, `starts a stopped agent only through its approval`) |
| **No network access at startup, none to hosted providers.** Ollama is probed on the loopback address only when the user opens Models or refreshes. | `crates/providers/src/ollama.rs`, `src/models/` | Workbench test (`looks for local providers when Models opens, not when Agents opens`); review |
| **A running program is not ended by accident.** Closing a terminal pane or tab, or quitting, asks first when a program other than the shell is in the terminal's foreground (read from the PTY with `tcgetpgrp`). | `crates/pty`, `src/workbench/workbench.ts` | Tests (`knows_when_a_job_is_in_the_foreground`, workbench tests) |
| Terminal command arguments are validated in Rust: sizes (1–4096 × 1–2048), session ids (`notFound` otherwise), and raw input only to an existing session | `crates/core/src/terminal.rs`, `crates/pty` | Unit and integration tests |
| No terminal content is persisted. Scrollback (10,000 lines) exists only in webview memory. Native output buffering is bounded by flow control (512 KiB per session). | `src/terminal/TerminalView.tsx`, `crates/pty/src/session.rs` | Tests (`output_pauses_until_acknowledged`) |
| OSC 52 clipboard writes and clickable links are off: the xterm.js add-ons that implement them are not installed | `package.json` | Review |
| Terminal processes do not outlive their session or the app. Close, reload, quit and crash all hang up the terminal, and a shell ignoring SIGHUP gets SIGKILL after 2 s. A closed session keeps reading its terminal until the end of output, so a shell that writes while exiting cannot hang (macOS waits for unread terminal output on the last close). | `crates/pty`, `src-tauri/src/lib.rs` | Tests (`a_shell_closed_while_starting_finishes_exiting`, …); manual `ps` checks after Cmd+Q, SIGTERM and Ctrl+D |
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
  **Enforced (Phase 4)** for the built-in definitions by a test; the approval
  dialog shows the exact command line. A per-workspace opt-in for such flags does
  not exist.
- **Prompt injection** from repository content, web pages or MCP tool results can
  steer an agent. The app cannot solve this. It reduces the impact by giving each
  agent session only the secrets it needs, not placing app-held secrets in the
  environment by default, and giving each agent a Git worktree of its own (built,
  Phase 5, docs/multi-agent.md), so
  changes can be reviewed before they touch the main checkout.
- **OS-level sandboxing** of agent sessions (macOS Seatbelt profiles, containers or
  VMs) is an explicit research item for Phase 12. Some agents already ship their
  own sandbox. The app should surface and prefer those rather than stack a
  second, incompatible one.

### 3.3 MCP tools

- **A stdio MCP server is arbitrary local code** with user privileges. Starting one
  is equivalent to running an installer. **Enforced (Phase 7):** the approval
  dialog shows the exact command line, the resolved executable path and the
  variables passed (by name and source) before first start, and again after any
  change; servers start only for a session of a trusted folder.
- **Unpinned versions** (`npx pkg@latest`, `docker … :latest`) let a server change
  under the user. Built-in and catalog definitions must pin versions (the test
  fixtures do). Pin by digest where the ecosystem allows it.
- **Tool poisoning.** Malicious tool descriptions can instruct the agent. Showing
  tool lists at enable time needs the app to act as an MCP client, which Phase 7
  deliberately does not; **not yet enforced** (deferred with the inspection
  client).
- **Rug pulls.** **Enforced (Phase 7)** for what the app starts: an approval pins
  the executable, every argument, the URL and the variables, and any change asks
  again. What a package manager fetches for an approved command (`npx pkg`) is
  not pinned by the app. The catalog (Phase 8) distributes no server definitions.
- **Remote MCP servers** receive workspace content through tool calls. HTTPS is
  required for non-loopback hosts, and URLs cannot carry credentials or
  environment references (enforced in validation, Phase 7). Authentication is the
  agent's own (OAuth); app-held tokens are **not yet** supported.
- **Project-scoped MCP configuration in a repository** (for example files that
  agents read automatically) must never be started by the app without workspace
  trust (§3.8).

### 3.4 Secrets and API keys

- **Storage:** macOS Keychain via the native secret store. No plaintext config
  files, and not `localStorage`. **Enforced (Phase 6, ADR 0014).**
- **Reference, don't embed.** Definitions use `SecretName`, and the type system
  makes embedding a value impossible (built). Validation rejects URLs with
  embedded credentials (built).
- **Delivery:** secrets are resolved at launch and placed only in the environment
  of the specific child that needs them. They are never set on the app process,
  so shells and other children cannot inherit them. **Enforced (Phase 6).**
- **Never in the webview.** No command returns a secret value. The UI can only set,
  replace or delete a secret and see whether one exists. The key the user types is
  sent once, to be saved, and the field is cleared. **Enforced (Phase 6).**
- **Never in logs, errors or crash reports.** `CommandError.message` must not include
  secret values or environment dumps (documented on the type). Every type that can
  hold a key or an environment prints redacted or names-only `Debug`. **Enforced
  (Phase 6).**
- **Precedence:** a session the app configures replaces the provider variables the
  shell sets, rather than mixing with them; one using the agent's own
  configuration gets no app-held key. **Enforced (Phase 6, ADR 0015).**
- **Residual risk:** once a secret is in an agent's environment, the agent (and
  anything it runs, which inherits its environment) can read and exfiltrate it,
  and other processes running as the user can read a process's environment
  (`ps -E`). Prefer agents' native login flows, where tokens stay in the agent's
  own store, over API keys the app injects; the agent's own configuration stays
  the default.
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
- Nothing is uploaded or indexed remotely by the app itself. Quick open and search
  walk the tree on demand and store nothing; there is no index. Search results
  can include files such as `.env` that are inside the workspace and not ignored:
  choosing the workspace grants that. Warning about sensitive files (`.env`,
  keys) is planned, not built.

### 3.6 External processes

- **PATH hijacking.** A GUI app's `PATH` differs from the user's shell. Agents are
  resolved to absolute paths through the user's login environment (absolute `PATH`
  entries only, so `.` never resolves into a workspace), and the resolved path is
  shown at approval and recorded with it. A binary appearing earlier on `PATH`
  later changes the resolved path, which requires approval again. **Enforced
  (Phase 4).**
- **Reading the login environment** runs the user's shell startup files once per
  app run, in the home directory (never a workspace), with no input, a 10 s timeout
  and process-group kill. This is the user's own configuration, as when they open
  a terminal. Agents receive that environment, including anything the user's
  startup files export; the app adds nothing of its own. **Phase 4.**
- **Orphans and runaway processes.** Every terminal process is a session leader
  tracked in a registry. Close, reload and quit send SIGHUP to it and its
  foreground job, then SIGKILL to its process group after a grace period. Jobs the
  user detached on purpose (`nohup`, `disown`) survive, as in any terminal.
  **Enforced (Phase 1).**
- **Environment leakage.** Terminal sessions inherit the app's environment plus
  `TERM`, `COLORTERM`, `TERM_PROGRAM` and, when no locale is set, `LANG`. Provider
  keys are never placed in the app's own environment, so shells cannot inherit
  them (tested, Phase 6). Under `pnpm tauri dev`, sessions also
  inherit the dev server's environment. **Phase 1 (terminal), Phase 4 (agents).**
- **Resource exhaustion.** Output is batched with bounded buffers and backpressure:
  a flood blocks the producer rather than growing memory. **Enforced (Phase 1).**
  Showing the number of sessions arrives with tabs.
- **Quitting.** Quit, closing the window and (on macOS) Quit from the Dock, logout
  and shutdown ask first while a terminal runs a program or a file is unsaved,
  then hang up every session and SIGKILL what remains after 500 ms. Force Quit,
  `kill -9` and crashes skip the question and the cleanup code; the kernel still
  hangs up every terminal when the app's side closes, so shells and their
  foreground jobs still end. A job that ignores SIGHUP and was started in the
  background of a terminal can survive a crash of the app, as it would survive
  closing any terminal emulator. **Enforced (Phases 1 and 3).**

### 3.7 Malicious integrations and the catalog

- **The catalog is untrusted metadata.** It cannot execute, start a server,
  change trust, grant approval or read a secret. It asks the system that owns an
  item, which applies its own checks (ADR 0018). **Enforced (Phase 8).**
- **Installation executes code.** The app installs nothing: no package managers,
  downloads or URLs. An agent that is not installed is shown as such.
  **Enforced (Phase 8).**
- **Supply chain of a future remote catalog:** typosquatting, compromised
  packages with install scripts, a compromised index. **Future:** signed metadata
  verified against keys the user trusts, pinned versions and checksums,
  provenance display, and installation only through the owning system's approval.
  Only the interfaces exist.
- **Skills steer agents, as any prompt does.** A user skill is as trusted as its
  author, and built-in skills are reviewed. A skill can grant nothing, and the
  agent's own permission prompts still apply. The approval dialog lists a
  session's skills.
- **No in-process plugins.** Third-party code never loads into the app process or
  the webview (architecture §14).

### 3.8 Workspace trust

Opening a folder must not execute anything from it. Before a workspace is trusted,
the app must not auto-start project-defined MCP servers, agent configurations,
tasks or hooks.

- **Built (Phase 3, ADR 0010):** every folder starts untrusted. The user can trust
  it, only through a native confirmation, and remove trust at any time. The
  decision is stored per exact folder, outside the folder, and shown in the status
  bar. The native side answers "is this workspace trusted?" in one place
  (`Workspaces::is_trusted`).
- **Enforced (Phase 4):** no agent starts in an untrusted workspace, and each
  agent also needs the user's approval in that exact workspace (ADR 0012).
  Removing trust removes the folder's approvals and stops its agents. Opening a
  folder still only lists and reads files and, when the user opens a terminal,
  starts their own login shell there.
- **Enforced (Phase 7):** MCP servers the app starts need trust and a
  per-workspace approval of exactly what runs; removing trust removes the
  folder's MCP approvals and stops its agents' servers. The app never starts
  repository-provided MCP configuration; a project's `.mcp.json` stays the
  agent's, behind the agent's own approval, in folders the user trusted.
- **Agent isolation (Phase 5)** is by working directory: each agent works in its
  own worktree, which prevents accidental interference through normal work. It is
  not a sandbox; an agent running as the user can still write elsewhere.
- **Honest limit:** trust is a statement by the user, not an analysis of the
  folder. It does not make a folder's contents safe, and it does not constrain what
  the user's own shell does there.

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
3. Never return a secret value through IPC, and never put one in a log line, an
   error message or a file the app writes. The only secret that crosses IPC is the
   key the user types, sent once to the native side to be saved.
4. Never render untrusted content as HTML in the main webview.
5. Never auto-execute anything from a workspace the user has not trusted.
6. Never add approval-bypass flags to an agent launch by default.
7. Pin versions in any built-in definition.
8. Do not describe a control as enforced until it is, and a test or check proves it.
