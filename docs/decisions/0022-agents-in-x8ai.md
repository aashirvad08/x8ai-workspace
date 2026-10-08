# 0022. Agents in `x8ai`

**Status:** Accepted

## Context

The terminal version (ADR 0020, 0021) has spaces, panes, tabs, the file list
and the editor. The app's reason to exist is agents: Claude Code, OpenCode and
Codex, each in a Git worktree of its own, only in trusted folders, only once
the user allowed it there (docs/agent-runtime.md, docs/multi-agent.md, ADR
0010, 0012, 0013). The questions for `x8ai`:

1. What does it share with the app?
2. Who answers "trust this folder?" and "allow this agent?", with no native
   dialog?
3. Which environment do agents get?
4. What is reviewed, and how, without the app's diff view?
5. What about sessions the app made with a model, MCP servers or skills?
6. Do agents stop when another space is shown?

## Decision

**The same runtime, stores and worktrees as the app.** `x8ai` uses
`x8ai-agents` (definitions, `plan`, `authorize`, `AgentRuntime`, worktree
`Isolation`) and the app's own stores: trust, agent approvals, and MCP
approvals (forgotten with trust). A worktree either made is found by the
other; an approval given in either covers both. Every run is authorized again
just before it starts (`Spaces::authorize`), as `agent_run` does.

**The Agents panel, in the sidebar (Ctrl-g a).** The folder's trust and
whether sessions get worktrees; the built-in agents, installed or not, allowed
or not; the space's sessions with their state. Enter on an agent starts a
session; on a session, shows its agent or runs it again. `c` reviews its
changes, `o` opens a shell in its worktree, `s` stops it, `d` removes it, `t`
trusts or stops trusting the folder, `r` takes an agent's approval back.

**The questions are `x8ai`'s, answered at the keyboard.** Trusting a folder
and allowing an agent ask in a box with the same content as the app's native
dialogs: the folder, the exact program, the model configuration, whether it is
isolated, and what it may do. Only `y` agrees. Nothing a program prints can
answer: a pane's output is parsed into its own screen and never reaches
`x8ai`'s input, which comes only from the user's terminal. Removing a session
says what goes (its worktree, its uncommitted changes) and what stays (a
branch with commits).

**Agents get `x8ai`'s own environment.** The app has no terminal, so it reads
the user's environment from an interactive login shell. `x8ai` is started from
that shell, so its environment already is the one the user's terminal gives a
program, `PATH` from `.zshrc` included, and it is used as it is (less Claude
Code's child-session marker, and the host terminal's variables, as for every
session). An interactive shell started from inside a terminal would stop,
waiting to own it: reading it the app's way took the 10 s timeout and failed.

**Review in a pager.** `c` writes what Git says changed (branch, commits,
files, and the diff, as `agent_changes` gives it) to a private temporary file,
with every control character of the diff shown as text (`^[`), never sent,
and our own colors added, then opens `less -R` on it in a tab. `less` comes
with macOS and has search; `q` closes the tab.

**This step runs agents with their own configuration.** A model, MCP servers
or skills chosen for a session come with step 4. A session the app made with
them is listed (marked `app`) but not run by `x8ai`: `AgentRuntime::run`
would refuse it, and running it without them would not be that session.

**Spaces keep their agents running.** In the app one workspace is open, and
opening another stops its agents. In `x8ai`, every space opened in a run keeps
its panes while another is shown (ADR 0020); agents are panes, so they keep
running too. Trust and approval are per folder, and stay as they were. Quitting
asks first while anything runs; stopping trust stops the folder's agents.

**An agent's pane stays when it ends.** Unlike a shell's: the session lives on
(docs/multi-agent.md), so the pane says how it ended and Enter runs it again,
with every check. Stopping it from the panel, or closing its pane, hangs it
up; its session and worktree stay.

## Consequences

- The workflow of the app's Agents view is in the terminal: start Claude Code
  in a worktree, watch it, review its changes, start another, remove sessions.
- Agents get what the user's terminal had when `x8ai` started. Variables a
  tool such as direnv set for the folder `x8ai` was started in reach agents in
  every space; the app avoids that by reading the shell in the home folder. A
  program installed after `x8ai` started is found once `x8ai` restarts.
- Approvals and worktrees are shared with the app; so is the rule that an
  agent runs only where the user trusted the folder and allowed it.
- Agents keep running in spaces not shown, until stopped, until trust is
  removed, or until `x8ai` quits.

## Alternatives considered

- **Reading the login shell as the app does, detached from the terminal:**
  needs `setsid` in the child, which only `unsafe` code can do here, or a PTY
  of its own, whose line discipline changes the output; and it would replace an
  environment `x8ai` already has with one read for no gain.
- **A diff view of `x8ai`'s own:** scrolling, search and colors that `less`
  already has.
- **Stopping a space's agents when another is shown:** the space's shells keep
  running; stopping only its agents would surprise more than it protects, and
  the permission (trust, approval) does not change by looking elsewhere.
