# Agent runtime

The agent runtime runs external coding agents (Claude Code, OpenCode, and later
others) inside a workspace. The app does not implement an agent, a model or
inference. An agent is an external program; the runtime finds it, asks whether it
may run, starts it in a terminal, and cleans up after it.

Built in Phase 4. Decisions: ADR 0012 (runtime, environment and approvals), ADR 0010
(workspace trust), ADR 0006 (terminal stack).

## Architecture

```
React (src/agents, src/terminal)          IPC                 Rust
┌─────────────────────────────┐   agent_list            ┌───────────────────────────────┐
│ Agents view: status, Launch │ ───────────────────────▶│ src-tauri/src/agents.rs       │
│ Workbench: trust → approval │   agent_request_approval│  definitions (built-in)       │
│ Terminal pane (agent kind)  │   agent_start           │  login environment (cached)   │
│   xterm.js + TerminalSession│   terminal_write/resize/│  trust + approvals (Workspaces)│
└─────────────────────────────┘   ack/close             └──────────────┬────────────────┘
                                                                       │
                                     crates/agents (no Tauri)          ▼
                                     plan ─▶ authorize ─▶ AgentRuntime::start
                                                                       │
                                     crates/pty: Sessions (shared with shells)
                                                                       │ PTY
                                                                       ▼
                                                        external agent process (claude)
```

| Piece | Where | Role |
| --- | --- | --- |
| `AgentDefinition` | `crates/core/src/agent.rs` | Data: id, name, description, `LaunchSpec` (program, args, env), requirements, capabilities, platforms |
| Built-in definitions | `crates/agents/src/builtin.json` | Claude Code and OpenCode. Adding an agent that needs no special handling is adding an entry. |
| Login environment | `crates/agents/src/environment.rs` | The environment an agent would have if typed in the user's terminal |
| Discovery | `crates/agents/src/discovery.rs` | The program, found on that environment's `PATH` |
| Runtime | `crates/agents/src/runtime.rs` | `plan` → `authorize` → `AgentRuntime::run` for an agent session; status, stop, cleanup |
| Isolation | `crates/agents/src/isolation.rs`, `crates/git` | A Git worktree per agent session (docs/multi-agent.md) |
| Approvals | `crates/workspace/src/store.rs` (`ApprovalStore`) | Which agent may run in which folder |
| Commands | `src-tauri/src/agents.rs` | `agent_list`, `agent_request_approval`, `agent_revoke`, and the session commands in docs/multi-agent.md |
| UI | `src/agents/` (view, store), `src/terminal/` (agent panes), `src/workbench/` (flow) | |

Nothing in the runtime is specific to one agent. Claude Code is the concrete
agent that validates it.

## The runtime API

The conceptual API maps onto existing pieces rather than a parallel stack. Since
Phase 5 an agent runs in an *agent session* (docs/multi-agent.md): a worktree of
its own in a Git repository, or the workspace itself otherwise.

| Operation | Implementation |
| --- | --- |
| `start(agent, workspace)` | `agent_create_session(agent)` then `agent_run(session, size, events)`: plan, authorize, spawn on a PTY session in the session's directory. The workspace is always the open one; the webview cannot name another. |
| `stop(agent)` | `agent_stop(session)`, or closing the agent's terminal: SIGHUP, then SIGKILL after 2 s |
| `restart(agent)` | `agent_run` again for the same session (Enter in its terminal, or Restart), with every check again |
| `getStatus(agent)` | `agent_list` (installed, approved) and `agent_sessions` (running, exited, failed, not running); natively `AgentRuntime::get` |
| `sendInput(agent, input)` | `terminal_write` on the agent's PTY session |
| `resize(agent, cols, rows)` | `terminal_resize` on the agent's PTY session |

## Lifecycle

```
not installed ──(user installs it, Refresh)──▶ installed
installed ──Launch──▶ [trusted?] ──no──▶ blocked: "not trusted" [Trust Folder…] ─▶ native trust dialog
                         │yes
                         ▼
                   [approved here, for this executable?] ──no──▶ native approval dialog ── Cancel ─▶ nothing runs
                         │yes                                          │Allow
                         ▼                                             ▼
                      starting ──spawn──▶ running ──exit 0──▶ exited
                         │                  │ ──non-zero / signal──▶ failed
                         └─spawn error──▶ failed        (Enter restarts: back to the checks)
```

The frontend walks the user through the steps; the native side enforces them.
`agent_create_session` and every `agent_run` refuse unless the open workspace is
trusted and the agent is approved there for the executable it would run now,
whatever the webview does.
`authorize` is the only way to obtain the `Authorized` value that
`AgentRuntime::run` takes, so a start without the checks does not compile.

## Trust requirements

Phase 4 is the first phase that enforces workspace trust (ADR 0010):

- An agent never starts in an untrusted workspace. Opening a folder is not
  permission to run anything in it.
- Removing trust from a folder removes its agent approvals and stops agents
  running in it.
- Trust is exact to one folder; so is approval.

## Approval model

Approval means "this agent may operate in this specific workspace", never "this
agent is approved everywhere".

```
Workspace (canonical root)
  ├─ trust            trusted-workspaces.json
  └─ agent approvals  agent-approvals.json
       ├─ claude-code: /Users/me/.local/bin/claude, args []
       └─ opencode:    …
```

- **What is stored:** per workspace root, each approved agent's id, the executable
  path and arguments that were shown when the user allowed it, and when. Nothing
  else.
- **Where:** `~/Library/Application Support/com.x8ai.workspace/agent-approvals.json`,
  mode 0600 in a 0700 directory, replaced atomically; a damaged file is set aside
  and approves nothing. Never in the project directory: a repository must not be
  able to approve agents for itself, and nothing the app reads from a workspace can
  change approvals (tested).
- **What it covers:** exactly one agent, launched as exactly that executable with
  exactly those arguments, in exactly that folder. Another folder, a folder inside
  or around it, another agent, a different executable earlier on `PATH`, or
  changed arguments are not covered and are asked about again.
- **Granting:** only in a native dialog (`agent_request_approval`) that shows the
  folder and the exact command line, which the webview cannot answer. If the open
  workspace changed while the dialog was up, nothing is approved.
- **Revoking:** the Agents view (Revoke), or removing the folder's trust.
- **Agents that update themselves:** the path is stored as found on `PATH`, not
  with symlinks resolved, so Claude Code updating itself in place (a symlink to a
  new version) keeps its approval. The approval is for a program at a location the
  user chose to install, not for a binary's content.

## Process model

- The agent is started natively by the runtime, directly, never through a shell:
  the resolved absolute executable, the definition's arguments, working directory
  = the session's worktree (or the workspace root without Git), and an explicit
  environment. The webview cannot choose any
  of these.
- It runs on a PTY session from the same `Sessions` registry as the user's shells,
  as a session leader in its own process group, so signals reach its children.
- It runs as the user, with the user's permissions: no elevation, no sudo, no
  extra directories, no injected credentials.

### Environment

A GUI app inherits launchd's minimal environment, whose `PATH` lacks
`~/.local/bin`, Homebrew and version managers. Terminals get the user's real
environment from their startup files; for example Claude Code's installer adds
`~/.local/bin` in `.zshrc`, which only an interactive shell reads. So the runtime
asks the user's shell for its environment once per app run (and again on
Refresh):

- `$SHELL -l -i -c '<fixed script>'`: the script prints `env -0` between two
  random markers, so anything the startup files print is ignored. The script is a
  constant; nothing is interpolated into it.
- It runs **in the home directory, never in a workspace**, so a repository's
  `.envrc` or similar is not read. It gets no input, a 10 s timeout, and its
  process group is killed if it hangs. Variables describing the terminal the app
  may have been started from (`TERM_SESSION_ID`, `TERM_PROGRAM`, …) are removed
  first, or macOS's `/etc/zshrc` would save and restore that Terminal window's
  session.
- The agent's environment is exactly that environment, minus the resolving
  shell's own state (`PWD`, `SHLVL`, …), plus the terminal variables every session
  gets (`TERM=xterm-256color`, `COLORTERM`, `TERM_PROGRAM`, `LANG` if unset).
  Nothing from the app's own environment is added.
- One variable is left out: `CLAUDE_CODE_CHILD_SESSION`. Claude Code sets it (with
  `CLAUDECODE`, its session id and the like) in the environment of every process
  it starts. When the app itself is started from inside a Claude Code session
  (`pnpm tauri dev` or `open` run there; macOS gives an opened app its caller's
  environment), the app inherits it, the login shell inherits it from the app,
  and so would the agent. A Claude Code that finds it takes itself for that
  session's child and saves no transcript ("Transcript saving is off — inherited
  CLAUDE_CODE_CHILD_SESSION marker"). An agent the app launches, or `claude`
  typed in one of its shells, is a session of its own, so the marker stops at
  every terminal the app starts (`x8ai-pty`, `Program::command`), and at an
  agent's planned environment (`plan`, in `crates/agents/src/runtime.rs`). The
  other variables pass as before.
- If the environment cannot be read, agents are looked up with the app's own
  environment and the Agents view says why.

This is what the user's own terminal would give the agent, including variables
their startup files export. Unless the user chose a provider and model for the
session (docs/models.md): then the agent's adapter replaces the variables it
controls with the app's configuration, including the provider's key from the
Keychain (ADR 0015). Otherwise the app adds no secrets.

### Discovery

The definition's program is looked up on that environment's `PATH`: the first
regular, executable file, in absolute `PATH` directories only (a relative entry
such as `.` would resolve against a workspace). Nothing is downloaded, installed
or fetched from the network; a missing agent is reported as not installed.

## Terminal integration

An agent pane is an ordinary terminal pane whose session is created with
`agent_start` instead of `terminal_create`. Everything else is unchanged: the same
xterm.js view, input as raw bytes, resize, flow control, scrollback limits,
Ctrl+C reaching the agent as SIGINT, splits (a split next to an agent runs a
shell). The tab is titled `Claude Code — <folder>`. When the agent exits, the pane
says how and offers Enter to restart it, which goes through every check again.

## Cleanup

| Event | What happens to the agent |
| --- | --- |
| It exits (or crashes, or is killed from outside) | The exit is detected and reported with its code or signal; the pane shows it |
| Its terminal closes | Asked first ("Stop Claude Code?"), then SIGHUP, SIGKILL after 2 s |
| Another folder opens | Asked first; the native side stops agents of any other workspace, and their panes close; their worktrees stay |
| The folder's trust is removed | Its agents stop, and their approvals are removed |
| The page reloads | Every session closes |
| The app quits (⌘Q, window close, Dock, logout) | Asked first; every session is hung up and killed after 500 ms |
| The app crashes or is force-quit | The kernel hangs up the PTY; the agent receives SIGHUP |

A closed session keeps reading its terminal until the end of output, so an agent
that prints while exiting cannot hang (see architecture §6).

## Security boundaries

| Boundary | Enforced by |
| --- | --- |
| The webview cannot choose what runs, where, or with what environment | Agent commands take an agent or session id, a size, and a provider id and model id (checked; the adapter decides what they mean) |
| No agent in an untrusted workspace | Native check on every start (`authorize`) |
| No agent without the user's approval for this folder, executable and provider | Native dialog to grant; native check on every start |
| A project cannot approve agents for itself | Approvals live in the app data directory; nothing reads approval state from a workspace |
| No hidden elevation, credentials or directories | The agent runs as the user, with the environment described above; a Keychain key only in a session whose provider the user chose |
| No orphaned agents | PTY session leaders, process-group teardown, quit and crash paths (tests) |

Not enforced, and not claimed: what an agent does once running. It has the user's
full privileges, like any program the user runs in a terminal. Agents' own
permission systems (Claude Code's tool approvals, for example) remain the control
inside the agent; the app never adds flags that bypass them (tested for the
built-in definitions).

## Testing

- `crates/agents/tests/runtime.rs`: the runtime against real processes on real
  PTYs, with a script standing in for an agent: discovery, every authorization
  rule, directory, environment, terminal, input, output, Ctrl+C, exit, crash,
  terminal close, app quit, and stopping on workspace change or untrust.
- `crates/agents/tests/environment.rs`: resolving the login environment with real
  shells, including startup files that print, hang or exit early.
- `crates/workspace/tests/store.rs`: approvals are exact, persistent, revocable,
  and cannot be granted by project files.
- `crates/agents/tests/live_claude.rs` (ignored by default): the same path with the
  real Claude Code, on a machine that has it:

  ```sh
  X8AI_LIVE_WORKSPACE=/some/scratch/folder \
    cargo test -p x8ai-agents --test live_claude -- --ignored --nocapture
  ```

  It never sends Claude a prompt; it only presses Ctrl+C and closes sessions.

## Future extension points

- **Models** [built: Phase 6, docs/models.md]: per-agent adapters turn "provider
  P, model M" into documented variables and flags, with the key from the Keychain
  in that one agent's environment. `EnvValue::Secret` in a definition is still
  refused.
- **MCP** [built: Phase 7, docs/mcp.md]: the adapter gives the agent its
  session's MCP servers through its documented per-session mechanism; stdio
  servers are started by the app when the agent connects, and stop with it.
- **Skills** [built: Phase 8, docs/catalog.md]: the adapter gives the agent its
  session's skills (instructions only): `--append-system-prompt` for Claude Code.
  OpenCode and Codex are unsupported.
- **More agents:** a definition in `builtin.json`, and an adapter for what the app
  should configure (Codex: its model). A future signed remote catalog could deliver more, through
  the runtime and its approvals (ADR 0018).
- **Structured agents:** a second runtime kind for agents with machine interfaces
  (ACP, headless JSON), reusing definitions, trust and approvals.
