# MCP servers

The user configures MCP servers once in the app. Agent sessions get them for
that session only, through each agent's documented per-session mechanism. The
app owns the servers' definitions, secrets and approvals, and the processes of
stdio servers, which belong to the agent session they were started for. The
agent remains the MCP client. The app never speaks MCP itself, never contacts an
HTTP server, and never installs or downloads anything.

Built in Phase 7. Decision: ADR 0017. Builds on the agent runtime (Phases 4–5,
docs/agent-runtime.md, docs/multi-agent.md) and the provider layer (Phase 6,
docs/models.md).

## Architecture

```
 MCP view ──add/edit/enable/remove, secrets──▶ mcp_* commands ──▶ mcp-servers.json (no secrets)
                                                                  Keychain (com.x8ai.workspace.mcp)
 Agents view ──launch(agent, model, session servers)──▶ agent_* commands
      1. which servers the session gets (global, this folder's, chosen)       x8ai-mcp::session
      2. what exactly each would run (command resolved on the login PATH)     x8ai-mcp::prepare
      3. trust + approval of exactly that (one native dialog)                 x8ai-mcp::authorize
      4. for stdio: a private socket per server                               x8ai-mcp::runtime
      5. the agent told where they are, for this session only                 x8ai-agents::adapter
      6. the agent runs; it connects; the app starts the server; stops it with the agent

 agent ──spawns──▶ bridge ⇄ ~/.x8ai/mcp/<session>-<token>/<n>.sock ⇄ app ──spawns──▶ stdio server
 agent ─────────────────────────── HTTPS ───────────────────────────────────────────▶ HTTP server
```

| Piece | Where |
| --- | --- |
| Registry entry, IPC types, validation | `crates/core/src/mcp.rs` (`McpServer`, `McpServerInput`, `McpServerStatus`, `SessionMcpServer`) |
| Registry, approvals, selection, environment, runtime, bridge | `crates/mcp` (`x8ai-mcp`, no Tauri) |
| Per-agent MCP configuration | `crates/agents/src/adapter/` (`AgentAdapter::mcp`, `configure_mcp`, `attach_mcp`) |
| Commands | `src-tauri/src/mcp.rs`, `src-tauri/src/agents.rs` |
| UI | `src/mcp/` (the MCP tab, ⇧⌘U), `src/agents/` (launch choices, session card) |

The Phase 0 `McpServerDefinition` (a catalog definition with a `LaunchSpec`) is
unchanged; it is what the catalog (Phase 8) will distribute. A registry entry is
what the user configured.

## Servers

```json
{
  "id": "github",
  "name": "GitHub",
  "description": "",
  "transport": { "kind": "stdio", "command": "npx", "args": ["-y", "@modelcontextprotocol/server-github"] },
  "env": [{ "name": "GITHUB_PERSONAL_ACCESS_TOKEN", "source": "secret" }],
  "enabled": true,
  "scope": { "kind": "global" }
}
```

- **id**: made natively from the name when the server is added (`github`,
  `github-2`), stable afterwards. Approvals, secrets and sessions refer to it.
- **transport**: one of:
  - `stdio`: a `command` and its `args`. The command is an absolute path or a
    program name looked up on the user's login `PATH`. The arguments are a list,
    each passed to the program as it is.
  - `streamableHttp`: a `url`.
- **env** (stdio only): variables by **name**, each with a **source**:
  - `secret`: a value saved in the Keychain for this server.
  - `inherit`: the variable of the same name in the user's login environment,
    when it is set.

  There is no literal value: a value typed into the registry could be a secret,
  and the registry is plain JSON.
- **enabled**: a disabled server is not attached to new sessions and not started
  for existing ones.
- **scope**:
  - `global`: every new session.
  - `workspace` (a folder root): new sessions in that folder.
  - `session`: only sessions it is chosen for at launch.

The registry is `mcp-servers.json` in the app's data directory (0600, replaced
atomically). A damaged file is moved aside to `.corrupt` and the registry starts
empty. An entry that no longer validates (a shell, an unknown field, a literal
value) is dropped, with a warning shown at startup. At most 100 servers.

### Validation

Checked before a server is stored, again when the registry loads, and before it
can be approved.

**stdio**
- The command is not empty. It is an absolute path without `.` or `..`, or a
  single program name. It is never a relative path, because that would resolve
  inside the workspace, which is the repository's code.
- A program name must not contain whitespace or shell characters
  (`;&|<>()$` and similar), so `npx -y server` is refused: its arguments go in
  the argument list.
- The command is never a shell (`sh`, `bash`, `zsh`, `dash`, `fish` and others).
  The app never runs a server through a shell, so nothing is ever interpolated.
- Arguments have no control characters. An argument that looks like a known
  credential (`ghp_…`, `sk-…`, `xoxb-…`, `AKIA…`) is refused: it belongs in a
  secret variable.

**Variables**
- Names match `[A-Za-z_][A-Za-z0-9_]*`, at most 128 characters, and each appears
  once.

**HTTP**
- The URL is a valid `http` or `https` URL, and `https` unless the host is this
  machine.
- It embeds no user or password and has no fragment.
- It has no query parameter named like a credential (`key`, `token`, `secret`,
  `password`, `auth`).
- It contains no `$`, `{` or `}`. Claude Code expands `${VAR}` and OpenCode
  expands `{env:VAR}` in server URLs; a URL containing either would send the
  agent's environment to the server.
- An HTTP server takes no variables: it gets nothing from this machine.

Protection against requests to internal addresses (SSRF) is not attempted beyond
these checks. The agent contacts the URL; the app never does.

## Transports

| Transport | Who starts it | Who talks to it | Status |
| --- | --- | --- | --- |
| stdio | the app, for a running session, when the agent connects | the agent, through the bridge | built |
| Streamable HTTP | nobody (it is a server somewhere) | the agent, directly | built |
| SSE | — | — | not supported (deprecated in MCP) |

## Lifecycle of a stdio server

1. **Nothing runs** because a server exists, because the app started, or because
   a session was created.
2. When a session's agent **runs** (`agent_run`), after trust and approval, the
   app makes a directory `~/.x8ai/mcp/<session>-<token>/` (0700) with one socket
   per stdio server (0600). The agent is told to run the **bridge** for each: the
   app's own executable with `--mcp-bridge <socket>`, a command that connects its
   stdin and stdout to the socket and knows nothing else.
3. When the agent **connects**, the app checks the connecting process: it must be
   the session's agent or one of its descendants (the peer process id, from
   `LOCAL_PEERPID` on macOS or `SO_PEERCRED` on Linux, walked up to the agent's).
   Any other process of the user gets the socket closed, and nothing starts. One
   connection at a time.
4. The app **starts the server**:
   - directly, without a shell: exactly the approved program and arguments;
   - in its own process group, in the session's directory;
   - with the environment described below.

   The server's stdin and stdout are connected to the agent. Its error output is
   drained into a bounded buffer (16 KiB), used only for a failure message, with
   every secret and inherited value redacted.
5. **Startup timeout**: if the server has not written anything within 30 s of the
   agent's first request, its process group is killed and the state says so.
6. **Crash**: the agent sees the connection close. The state says how the server
   ended ("exited with code 3: …"). The agent may reconnect, which starts the
   server again, at most 5 times per run of the agent; after that it is not
   started again until the agent restarts. The app never restarts a server on its
   own.
7. **End**: when the agent's process is gone, however it ended. It may have
   exited or crashed, been stopped, had its terminal closed, or had its workspace
   closed or untrusted. The run watches the agent's process id, because stopping
   an agent hangs up its terminal without reporting an exit. The run also ends
   when the page reloads and when the app quits. Every server's process group gets
   SIGTERM, then SIGKILL after 2 s. The sockets and their directory are removed.
   A server that ends by itself (the agent closed its stdin) takes its process
   group with it.
8. If the app **crashes**, servers lose their stdin and exit, as the MCP stdio
   transport requires. The next start of the app removes any socket directory
   left behind. It starts nothing.

HTTP servers have no lifecycle in the app. The app passes the URL to the agent,
which connects when it needs to.

## Environment policy

A stdio server's environment is built by the app, not inherited from the agent.
The agent's environment may hold the provider key the app configured for the
session, and the user's login environment may hold any number of keys. Exactly:

1. **Base**: `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `TMPDIR`, `LANG`, `TZ`
   and `LC_*`, from the login environment, when set. Enough to find `node`,
   `npx`, `uvx` and similar.
2. **Inherited**: the server's `inherit` variables, from the login environment,
   when set there. The user listed each one, and the approval showed it.
3. **Secrets**: the server's `secret` variables, from the Keychain. If one is not
   saved, the server is not started ("secret not saved"), rather than started
   half configured.

Nothing else reaches a server:
- not the session's provider environment (`ANTHROPIC_API_KEY`, `OPENAI_API_KEY`,
  `OPENCODE_CONFIG_CONTENT`…);
- not the agent's own variables;
- not a provider or cloud key from the shell unless the user listed it as
  `inherit`;
- not another server's secrets.

`ServerEnvironment` prints variable names only.

## Scopes and sessions

| Scope | Attached to |
| --- | --- |
| global | every new session of an agent that can use it |
| workspace | new sessions in that folder |
| session | a new session it is chosen for at launch (checkboxes on the agent card) |

- A session's servers are fixed when it is created. They are recorded, by id, in
  the session and in its worktree's metadata (`"mcp": ["github", …]`, never a
  command or a secret), so they survive restarts.
- **A session never gains a server.** A server enabled or added later goes to new
  sessions only.
- **A later run uses what the session still has:** servers that still exist, are
  enabled, still belong to its folder, have their secrets and a command that is
  found. The others are left out, and the session card says why ("disabled",
  "removed from the app", "secret not saved: …").
- The session card lists each server and what it is doing: not running, ready,
  running (pid), ended, failed (why), remote, not used (why). Never a secret.

## Secrets

Keychain service `com.x8ai.workspace.mcp`, one generic password per server
variable, account `<server id>/<VARIABLE>`. This is the same mechanism and the
same rules as provider keys (ADR 0014, docs/models.md):
- the webview sends a value once, to save it (`mcp_set_secret`), and the field is
  cleared;
- no command returns a value; the webview sees only `missing` or `inKeychain` per
  variable, and `configured` for the server;
- a value is read natively only to start the server, and placed only in that
  server's environment: never the agent's, never a file;
- removing a server, or removing a secret variable from it, deletes the saved
  value.

## Approval

An MCP server runs only in a trusted workspace, and only once the user approved
exactly what would run, in that workspace (`mcp-approvals.json`, outside every
workspace). ADR 0015's approval rules apply; this extends them.

| Change | Needs a new approval |
| --- | --- |
| The executable (another command, or the same name found elsewhere on `PATH`) | yes |
| Any argument | yes |
| The URL | yes |
| A variable added, removed or renamed, or its source changed | yes |
| The name, description, scope, enabled state | no |
| A secret's value (rotation) | no |
| Another agent using it in the same folder | no: the approval is for the server in the folder |

- **The dialog** is one native dialog for everything not yet approved at launch:
  the agent (Phases 4–6), and each server with its name, transport, the exact
  command and arguments or the URL, its variables by name and source, and the
  workspace. It never shows a secret. A changed server is asked for again ("changed
  since it was allowed").
- **Before an existing session runs again** (Terminal, Restart), the workbench
  asks for whatever it would now run that is not approved
  (`agent_request_session_approval`).
- **Untrusting a folder** removes its MCP approvals, and stops its agents and
  with them their servers.
- **Removing a server** forgets its approvals everywhere.

Creating a session, and every run, check trust and approval natively. The runtime
can only be started with an `Authorized` set, made by `authorize` from exactly the
prepared servers, and every launch must match one of them.

## Agent adapters

The existing `AgentAdapter` trait gained two methods, unsupported by default:
- `mcp()`: whether the app can give the agent MCP servers;
- `configure_mcp(servers, env)`: the variables and arguments that do it.

`attach_mcp` applies them to a launch. It refuses a transport the agent's
definition does not declare (`capabilities.mcpTransports`). Servers are named
`x8ai-<id>` for the agent, so they cannot collide with the agent's own.

| Agent | Mechanism (documented) | Global configuration |
| --- | --- | --- |
| Claude Code | `--mcp-config '{"mcpServers": {"x8ai-github": {"type": "stdio", "command": <bridge>, "args": ["--mcp-bridge", <socket>]}, "x8ai-docs": {"type": "http", "url": …}}}'`; not `--strict-mcp-config`, so the user's and the project's own servers stay theirs | `~/.claude`, `~/.claude.json` never written by the app |
| OpenCode | an `mcp` key (`{"type": "local", "command": [<bridge>, "--mcp-bridge", <socket>], "enabled": true}` or `{"type": "remote", "url", "enabled": true}`) merged into `OPENCODE_CONFIG_CONTENT`: the app's inline configuration if it configured a provider, or the shell's, or a new one | OpenCode's files never written |
| Codex, others | no adapter: reported as not supported; a session gets no servers, and choosing one for it is refused | untouched |

The agent's MCP configuration holds only the bridge and a socket path, or a URL:
no command of the user's, no variable, no secret. The flags added
(`--mcp-config …`) are not part of the agent's approval: the servers they point
to are approved separately, as above.

Claude Code starts its MCP servers only once the folder is trusted in its own
trust prompt; until then, nothing connects and nothing starts.

## Restart

- On start, the app loads the registry and the approvals and removes stale
  sockets. **No server starts.**
- Secrets stay in the Keychain.
- Sessions found again from their worktrees keep their server ids. Their agents
  are not running; the next run goes through trust and approval like any other,
  and rebuilds the configuration from the registry as it is then.

## IPC

| Command | Does |
| --- | --- |
| `mcp_list` | Every server: its entry, each secret's state, whether it is ready, why not, and which agents can use it |
| `mcp_add(server)` / `mcp_update(id, server)` | Validated natively; a workspace scope is the open folder |
| `mcp_set_enabled(id, enabled)` | |
| `mcp_remove(id)` | Also deletes its secrets and forgets its approvals |
| `mcp_set_secret(id, name, value)` / `mcp_remove_secret(id, name)` | Only for the server's `secret` variables; returns the status, never the value |
| `agent_request_approval(agent, model, mcp)` | One dialog for the agent and the servers a new session would get |
| `agent_request_session_approval(session)` | The same, for an existing session before it runs again |
| `agent_create_session(agent, model, mcp)` | The session and its worktree record the attached server ids |
| `agent_run(session)` | Starts the session's servers' sockets, then the agent |

`mcp` lists session-scoped servers chosen at launch, by id. The webview never
supplies a command, a path, a socket or a value.

## Files

| File | Holds | Never holds |
| --- | --- | --- |
| `<app data>/mcp-servers.json` (0600) | servers: names, command and arguments or URL, variable names and sources, enabled, scope | secret values |
| `<app data>/mcp-approvals.json` (0600) | per folder: server id, resolved executable, arguments or URL, variables by name and source | secret values |
| Keychain, `com.x8ai.workspace.mcp` | secret values | — |
| `~/.x8ai/worktrees/…/<name>.json` | the session's server ids | configuration, secrets |
| `~/.x8ai/mcp/<session>-<token>/*.sock` | sockets, while the agent runs | anything else |

## Known limitations

- **The socket is for the user's processes.** The directory is 0700 and a
  connection must come from the session's agent or a descendant. That check is by
  process ancestry, so a process the agent itself runs (a tool command) can also
  connect, as it could run the server's command itself.
- **Servers run as the user**, in the session's directory, with the user's
  privileges and network. The app controls what starts and its environment, not
  what the server does. OS sandboxing is Phase 12.
- **HTTP server authentication** is the agent's (for example Claude Code's own
  OAuth for remote servers). The app stores no header tokens for HTTP servers.
- **No test button.** The app does not start a server or contact a URL just to
  check it. A server is exercised only by an approved session.
- **OpenCode** support is implemented from its documentation and unit-tested, not
  verified against a running OpenCode.
- **Codex** is not a built-in agent and has no adapter.
- Duplicate-server resolution inside an agent (the user's own server with the
  same command) follows the agent's rules; the app's are named `x8ai-<id>`.
- SSE servers, header secrets, and servers from a catalog (Phase 8) are not
  supported.

## Tests

- `crates/core/src/mcp.rs`:
  - a command is one program, never a command line, a relative path or a shell;
  - arguments are passed as they are;
  - credentials are refused in arguments and URLs;
  - URL rules;
  - variables are names with a source;
  - an entry serializes without values.
- `crates/mcp/tests/registry.rs`:
  - creation and persistence;
  - a damaged file, and bad entries dropped;
  - add, edit (id kept), enable and disable, remove;
  - invalid servers refused before anything is stored;
  - global, workspace and session scopes;
  - a later run never gains a server;
  - the file holds names and sources only.
- `crates/mcp/tests/approvals.rs`:
  - trust and approval are required, per workspace;
  - a changed command, argument, variable or source, the same name found
    elsewhere on `PATH`, or a changed URL needs a new approval; cosmetic changes
    do not;
  - a missing command is not resolved in the workspace;
  - the environment is the base plus the server's own variables, never provider
    keys or other servers' secrets;
  - a missing secret refuses the start;
  - secrets are in the store only.
- `crates/mcp/tests/runtime.rs`, with real processes, the test server and the
  real bridge:
  - a server starts only when the session's agent connects;
  - a connection from outside the session starts nothing;
  - it gets exactly the approved argv and environment;
  - stopping the session ends it and everything it started, and removes the
    sockets;
  - it ends when the agent disconnects;
  - the run ends when its agent's process is gone, however it ended;
  - a crash is reported with redacted output, and restarts are bounded;
  - the startup timeout;
  - sessions and several servers per session are isolated;
  - quitting leaves nothing running;
  - only exactly what was approved can start;
  - a new runtime starts nothing and sweeps leftovers;
  - a late stop of an old run spares the new one;
  - the secret is not written anywhere.
- `crates/agents/tests/mcp.rs`:
  - Claude Code's `--mcp-config`;
  - model and MCP together;
  - OpenCode's inline configuration merged three ways;
  - an agent without support gets nothing, with the reason;
  - an unsupported transport is refused;
  - a session keeps its servers, a run may drop but never add one;
  - nothing global is written;
  - worktree metadata keeps ids only, and tampered ids are ignored.
- `src-tauri/src/mcp.rs`: the status sent to the webview never holds a secret.
- Frontend:
  - the client sends a secret only to save it and has no way to read one or to
    start a server;
  - launch choices;
  - add, secret, remove (asks), and refusals, without repeating a secret;
  - launch with chosen servers;
  - approval before an existing session runs again.
- Live (`crates/mcp/tests/live_claude_mcp.rs`, `--ignored`): the real Claude Code
  with the test server; see the Phase 7 report.
