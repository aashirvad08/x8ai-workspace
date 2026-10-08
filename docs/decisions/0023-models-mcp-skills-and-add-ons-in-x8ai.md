# 0023. Models, MCP, skills, the catalog and add-ons in `x8ai`

**Status:** Accepted

## Context

`x8ai` runs agents (ADR 0022), but only with their own configuration. The app
also gives a session a model from a provider whose key is in the Keychain
(ADR 0014, 0016), MCP servers it runs and owns for that session (ADR 0017),
and skills (docs/catalog.md); its catalog lists all of them (ADR 0018); and a
space's terminals get the add-ons added to it (ADR 0019). A session the app
made with a model, MCP servers or skills was listed in `x8ai` but not run. The
questions for `x8ai`:

1. What does it share with the app, and where does the shared logic live?
2. How are keys and secrets typed, with no native field, and never shown?
3. Without the app's launch dialog, how is a model, an MCP server or a skill
   chosen for a launch, and what is asked about it?
4. Who runs a session's MCP servers, and how does the agent reach them?
5. How is an add-on installed from inside the terminal?
6. How is it tested without touching the user's Keychain?

## Decision

**The app's stores and Keychain items, and the app's rules from the crates.**
Providers and model ids (`providers.json`), MCP servers and their approvals,
skills and spaces with their add-ons are the app's files, read when needed and
written straight back. API keys and MCP secrets are the same Keychain items:
their service names now live in `x8ai-providers` and `x8ai-mcp`, so the two
hosts cannot name different ones. What both hosts need moved out of
`src-tauri` into the crates: which MCP servers a session gets
(`x8ai_mcp::selection`), and a space's terminal environment for its add-ons
(`x8ai_addons::space`). The app calls them as `x8ai` does.

**Four list panels, in the sidebar like the Agents panel.** Ctrl-g `m` Models,
`u` MCP, `k` Catalog, `e` Add-ons. Each lists rows with their state and shows
its keys underneath; a click or ↑↓ selects.

- Models: each provider, whether its key is saved, its models. `s` saves a
  key, `a` adds a model id, `d` removes either, `r` looks for local models
  (Ollama, on this Mac only), Enter gives a model to an agent's next launch.
- MCP: each server with its state, transport and scope. `n` adds one, Enter
  edits it, space turns it on or off, `s` saves a secret, `d` removes it, `l`
  gives a session server to an agent's next launch. A variable is typed as
  `NAME` (a secret, kept in the Keychain) or `NAME=shell` (taken from the
  environment); arguments are split as typed, never by a shell.
- Catalog: agents, models, MCP servers and skills in one list, with the
  statuses `x8ai_catalog::assemble` gives the app. `/` filters; Enter does the
  obvious thing (an agent's panel, a model or session server or skill for a
  launch); `n`, `e`, `d` write, change and remove the user's own skills.
- Add-ons: the space's add-ons and what is installed. Enter adds one; `d`
  removes it; `s`, or `/share <space>` on the Welcome, gives another space the
  same ones.

**Keys and secrets are typed into a form that shows dots.** A secret field
keeps what is typed and draws one dot per character; pasting works. The value
goes to the Keychain when the form is sent, and is never drawn, listed, logged
or kept: only starting an agent or an MCP server reads it, for that program's
environment, and the Keychain is asked at most once per run for each item. A
key the app saved is in the app's Keychain item, so macOS may ask once whether
`x8ai` may read it.

**Choices for a launch, instead of the launch dialog.** Each agent has, per
space, what its next launch gets: a model, session MCP servers, session
skills. They are chosen from the panels or on the agent's row in the Agents
panel (`m`, `u`, `l`), and shown under it (`→ claude-opus · 1 MCP · 1 skill`).
Enter launches with them. The approval question, answered only at the
keyboard (ADR 0022), lists the model and its provider, every MCP server with
its transport and its exact command or URL, and the skills; `y` allows the
agent and those servers in this folder. The session records them, and running
it again uses the recorded ones, checked again. Sessions the app made with a
model, MCP servers or skills now run in `x8ai` too.

**`x8ai` runs its sessions' MCP servers itself.** It has an `McpRuntime` of
its own, as the app does: a session's servers start with its agent, get their
secrets in their environment, and stop when it ends. The agent reaches each
through a bridge, which is the `x8ai` binary started with `--mcp-bridge
<socket>`, as the app uses its own binary. The sockets are in
`~/.x8ai/mcp-<pid>`, not the app's `~/.x8ai/mcp`: each host removes its
folder's sockets as stale, and two runs must never remove each other's. The
path is short because a socket's path has at most 104 bytes on macOS. The
folder goes when `x8ai` quits, and folders of an `x8ai` that is gone are
removed when the next one starts.

**An add-on is installed in a tab.** Adding one that is not installed asks
first, showing the exact Homebrew command; `y` runs it in a tab of its own,
where it can be watched, and the add-on is added when the tab ends well and
the tool is found. Nothing is installed without that `y`. New terminals in a
space get its add-ons' environment from the same code as the app's.

**Tests never touch the Keychain.** In debug builds only, `X8AI_TEST_SECRETS`
points secrets at a file and `X8AI_TEST_MCP_SOCKETS` puts sockets in a short
folder; release builds ignore both. End-to-end tests run the real binary on a
PTY with a stand-in Claude Code that prints the model and whether the key is
set, finds the skill in its prompt, and talks to an MCP server through the
bridge, which answers with its secret.

## Consequences

- What the app configures, `x8ai` uses, and the other way round: a key saved
  in either, a server added, a skill written, an add-on added.
- An agent's next launch is set up before it, with the keys shown on its row,
  rather than in one dialog; the approval still shows everything at once.
- `x8ai` and the app each run the MCP servers of the sessions they started,
  and stop them with those sessions.
- macOS may ask, once, before `x8ai` reads a key the app saved (and again
  after an update changes the binary).
- `brew` runs only after the user agreed to the exact command it shows.

## Alternatives considered

- **One launch dialog with every choice, as in the app:** a form with a list
  of models and two lists to tick does not fit a terminal of 80 columns, and
  the choices are wanted again for the next launch.
- **Sharing the app's MCP sockets folder:** each host would remove the other's
  live sockets as stale.
- **Reading secrets from the environment, or a file, in release builds:** a
  second place for keys, outside the Keychain (ADR 0014).
- **Installing add-ons without a tab:** the user could not watch Homebrew, or
  read why it failed.
