# 0017. MCP servers managed by the app, owned by agent sessions

**Status:** Accepted (Phase 7)

## Context

Phase 7 lets the user configure MCP servers once and give them to agent sessions.
An MCP server is powerful: a stdio server is a program running as the user, and
any server receives what the agent sends it. Five questions:

1. **Who owns MCP configuration**: each agent's own files, or the app?
2. **Where do MCP secrets live?**
3. **Who starts a stdio server**, and whose process is it?
4. **How does an agent get its servers** without the app changing the user's
   global agent configuration?
5. **Where do servers come from**: typed in by the user, or discovered and
   installed from the internet?

The login shell may carry the user's provider and cloud keys (the Phase 6
lesson), and the agent's own environment carries the provider key the app
configured. If the agent started stdio servers itself, they would get whatever
the agent passes down, and Claude Code does not document which variables a
stdio server inherits.

## Decision

**The app manages MCP servers.** They are registered in the app
(`mcp-servers.json`, no secrets), enabled or disabled, scoped (every session,
one folder, or chosen per session), approved per workspace for exactly what
they would run, and given to agent sessions through each agent's adapter. The
agent remains the MCP client. The app does not speak MCP and does not contact
HTTP servers.

**Secrets use the Keychain** (ADR 0014), one item per server variable. Only
names and sources are stored anywhere else. The webview can save and delete a
value, never read one.

**Stdio servers are started by the app and belong to the agent session.** The
agent is given a bridge instead of the server's command: the app's own
executable with `--mcp-bridge <socket>`, which connects stdin and stdout to a
private socket of the session.

- When the session's agent connects (checked by process ancestry), the app starts
  the approved program directly, with an environment it builds: a base of `PATH`,
  `HOME`, locale and the like, the server's own inherited and secret variables,
  and nothing else.
- The app tracks the process, enforces a startup timeout, bounds its error
  output, limits restarts, and kills its process group when the agent's process
  is gone (however it ended) or the app quits.
- Nothing starts at app startup, or because a server or a session exists.

**Per session, through documented mechanisms only.**
- Claude Code: `--mcp-config` with a JSON string.
- OpenCode: an `mcp` key in `OPENCODE_CONFIG_CONTENT`.
- Agents without an adapter (Codex) are reported unsupported and get nothing.

The configuration names only the bridge and a socket path, or a URL. `~/.claude`,
`~/.claude.json`, OpenCode's and Codex's files are never written. The user's and
the project's own MCP servers are left to the agent, as before.

**Approvals pin what runs.** The resolved executable, every argument, the URL,
and the variables by name and source are pinned. Changing any of these asks
again. The name, description, scope, enabled state and secret values do not.
Approvals are per workspace, outside it, and removed when its trust is.

**No discovery, installation or downloads.** Servers are typed in by the user.
A command must already be installed (the user's `npx` may fetch a package; that
is the user's configured command, shown in the approval).

## Consequences

- An MCP server never receives the session's provider key, an unrelated key from
  the shell, or another server's secrets, whatever the agent would have passed.
  This is verified with the real Claude Code.
- Server processes are the app's to account for: no orphans after a stop, a
  crash of the server, or quitting. A crash of the app closes their stdin, and
  the next start removes stale sockets.
- The bridge adds one small process per connected stdio server, and a socket
  path in the agent's MCP configuration.
- A server is started lazily, once per connection. The agent reconnecting starts
  it again, bounded per run; a server holding state across connections loses it.
- Any process the agent itself starts can also connect to the session's sockets.
  It could equally run the command itself; the socket does not widen what the
  agent can reach.
- Claude Code connects its servers only after its own folder-trust prompt is
  answered; until then, nothing starts.
- HTTP servers authenticate with the agent (Claude Code's OAuth, for example). The
  app holds no header tokens yet.

## Alternatives considered

- **Let the agent start stdio servers** from their real command, with secrets in
  its configuration or environment. The server's environment would be whatever
  the agent passes: provider keys included. There would be no lifecycle control,
  and secrets would sit in the agent's argument list or configuration. Rejected.
- **Write the servers into the agent's configuration files** (`~/.claude.json`,
  `.mcp.json`, `opencode.json`). This changes the user's or the project's global
  configuration, outlives the session, and can end up in a commit. Rejected.
- **An MCP gateway in the app** (one endpoint multiplexing all servers). It makes
  the app an MCP client and proxy, on the path of every tool call. Deferred: it
  may be worth it for policy and audit later, with its own ADR.
- **`nc -U` as the bridge.** macOS `nc` does not exit when the socket closes, so
  the agent would not notice a crashed server. Rejected for a bridge of the app's
  own.
- **Starting servers eagerly** when the session starts. They would run without
  being used, and with nothing to talk to. Rejected for a start on connection.
- **Remote discovery, a marketplace, automatic installation.** Out of scope: the
  catalog (Phase 8) will distribute pinned, reviewed definitions. Until then, a
  server runs only if the user typed its command and approved it.
