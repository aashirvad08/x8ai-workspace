# 0012. Agent runtime: PTY sessions, the login environment, and per-workspace approval

**Status:** Accepted (Phase 4). Amended by [0015](0015-environment-precedence.md): an approval also pins the provider and endpoint an app-configured session uses.

## Context

Phase 4 runs external coding agents (Claude Code first) inside a workspace. The
app must not implement an agent, must not be coupled to one, and must enforce the
workspace trust recorded since Phase 3. Five questions shaped the design.

1. **What runs the agent's terminal?** Agents are interactive terminal programs.
   A second terminal implementation would duplicate the flow control, cleanup and
   hangup guarantees of `crates/pty` (ADR 0006) and drift from them.
2. **Which environment does the agent get?** A GUI app inherits launchd's minimal
   environment. On this machine, `claude` is found only by an *interactive* login
   shell, because its installer adds `~/.local/bin` in `.zshrc`. The user expects
   the agent to behave as if they had typed its name in their terminal.
3. **Which executable is it?** A name like `claude` resolves differently as `PATH`
   changes. The user should approve what will actually run.
4. **What does approval mean, and where does it live?** It must be scoped to one
   workspace, survive restarts, and be impossible for a repository to grant.
5. **How is trust enforced** so that no code path can skip it?

## Decision

**Agents are PTY sessions.** `AgentRuntime::start` spawns the agent with
`Program::Exec` on the same `Sessions` registry as the user's shells. An agent
session is driven with the existing `terminal_*` commands and shown in an ordinary
terminal pane. The only PTY change is that `Program::Exec` can take an exact
environment. Status, stop-on-workspace-change and stop-on-untrust are tracked by
the runtime on top of the sessions.

**The environment is the user's login environment, read once.** The runtime runs
`$SHELL -l -i -c` with a constant script that prints `env -0` between random
markers. It runs in the home directory (never a workspace), with no input, a 10 s
timeout and process-group kill, and without the host terminal's variables. The
agent gets exactly that environment plus the standard terminal variables, and
nothing from the app's own environment. Agents are started directly, not through
a shell, so what runs is exactly the planned command.

**The executable is resolved natively and pinned by the approval.** The program is
looked up on that `PATH` (absolute directories only). The resolved path, as found
(symlinks not resolved), and the arguments are what the approval dialog shows and
what the approval records. A different executable or different arguments need a
new approval. An agent that updates itself in place keeps its approval.

**Approval is per workspace, per agent, in the app's data directory.**
`agent-approvals.json` (0600, atomic, damaged files set aside) holds, per
canonical workspace root, the approved agents with their executable and
arguments. It is granted only in a native dialog (`agent_request_approval`) and
revoked from the Agents view or by removing the folder's trust. Nothing is read
from the workspace.

**Enforcement is structural.** Starting requires an `Authorized` value, which only
`authorize(plan, trust, approvals)` creates, and only when the workspace is trusted
and the exact launch is approved. `agent_start` computes the plan from the open
workspace and the built-in definition (the webview passes only an agent id and a
terminal size) and authorizes it on every start, including restarts.

**Agents do not outlive their workspace's permission.** Opening another folder
stops the previous folder's agents (natively, after the UI asks). Removing trust
stops the folder's agents and forgets its approvals. Running agents count as busy
for close and quit confirmations.

## Consequences

- One terminal stack for shells and agents: every PTY fix and guarantee applies to
  both, including hangup, SIGKILL fallback, drain-on-close and quit cleanup.
- Agents see the same `PATH` and exported variables as in the user's terminal,
  including anything their startup files export. The app adds no secrets.
- Reading the environment runs the user's shell startup files once per app run
  (and on Refresh), invisibly, in the home directory. A startup file that hangs
  costs up to 10 s, then the app's own environment is used and the Agents view
  says so.
- Approval is keyed by path, not content: replacing the binary at the approved
  path keeps the approval. That is where the user installed their agent, and
  anything that can write there can already run as the user.
- The approval survives restarts and app updates; built-in definition changes
  that alter arguments require approval again.

## Alternatives considered

- **Launch through the user's shell** (`$SHELL -l -i -c 'exec "$0" "$@"' claude`).
  Gives the terminal environment for free, but runs startup files in the
  workspace (where a `.envrc` hook would apply) on every launch, prints their
  output into the agent's terminal, and lets the shell resolve the program at
  exec time instead of the path the user approved. Rejected.
- **Only the login `PATH`, not the whole environment.** Predictable, but the agent
  would behave differently from the user's terminal (proxies, CA bundles, the
  agent's own configuration variables). Rejected for the full environment,
  documented.
- **A non-interactive login shell** (`-l` without `-i`). Misses `.zshrc`, where
  common installers add their `PATH` entries; Claude Code was not found that way
  on this machine. Rejected.
- **Pin approval to a content hash.** Every self-update of the agent would need
  approval again. Deferred to the catalog (Phase 10), where definitions carry
  hashes.
- **Approval inside the project** (a file in the repository). A repository could
  approve agents for itself. Rejected.
- **A separate agent process manager** (not PTY-based). Agents are interactive
  terminal programs; it would need a terminal anyway. Rejected.
