# x8ai Workspace

**A workspace for coding with AI agents, in your own terminal, on macOS.**

`x8ai` turns the terminal you already use into a workspace: your shells in tabs
and split panes, your project's files, your editor, and the coding agents you
already use, such as Claude Code, OpenCode and Codex. Pick an agent, choose the
model it should use, and let it work. Each agent gets its own copy of your
repository, so it never touches your checkout, and you review what it changed.
Close the terminal and everything keeps running; run `x8ai` again and it's all
there.

```sh
brew install aashirvad08/tap/x8ai
x8ai
```

x8ai doesn't come with its own AI. It runs the agents and models you choose,
and it doesn't favor any vendor.

---

## What you can do

- **Start from the Welcome screen.** Type `/cd` to open a folder as your space,
  `/new` for a new one, or `/home` for your home folder.
- **Work in real terminals.** Your own shell, in tabs and split panes, with the
  mouse: click, drag the line between panes, scroll, select to copy.
- **Browse your code and edit it.** A file list of the folder, and your own
  editor (`$EDITOR`, such as vim, Neovim or Helix) one keypress away.
- **Run coding agents safely.** Agents run only in folders you trust, and only
  after you approve the exact program and settings. In a Git repository each
  agent session works in a Git worktree of its own, on its own branch.
- **Run several agents at once** and review what each one changed, commit by
  commit and file by file, before you merge anything.
- **Choose the model.** Save an API key once for Anthropic, OpenAI, Google or
  OpenRouter, or use local models with Ollama, and pick a model for each launch.
- **Give agents tools with MCP servers.** Add a server once and attach it to the
  sessions that need it.
- **Attach skills.** Skills are reusable instructions, such as "write a failing
  test first", that you give to a session.
- **See everything in one place.** The Catalog lists every agent, model, MCP
  server and skill, whether it's ready, and what it still needs.
- **Set up a space's terminals with add-ons**, such as Starship, fzf or
  ripgrep, without editing your `~/.zshrc`.
- **Close the terminal; nothing stops.** Your shells and agents keep running in
  the background, and `x8ai` brings them back, in this terminal or another.

## Supported agents and providers

| Agent | Choose its model in x8ai | MCP servers | Skills |
| --- | --- | --- | --- |
| [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | Anthropic, OpenRouter, Ollama | ✓ | ✓ |
| [OpenCode](https://opencode.ai) | Anthropic, OpenAI, Google, OpenRouter, Ollama | ✓ | — |
| [Codex](https://github.com/openai/codex) | OpenAI | — | — |

Any agent can also use its own configuration, exactly as it does in your
terminal. You install the agents yourself; x8ai finds them on your `PATH`. Any
other command-line tool runs in x8ai's shells as usual.

---

## Install

**You need** macOS 13 or later (Apple silicon or Intel) and
[Homebrew](https://brew.sh).

```sh
brew install aashirvad08/tap/x8ai
x8ai
```

**Install an agent** if you don't have one yet, for example Claude Code,
OpenCode or Codex, following its own instructions. x8ai doesn't install them.

x8ai works in any terminal: Terminal, iTerm2, Ghostty, kitty, WezTerm, the
terminal in your editor. Colors are richest in one that shows 24-bit color.

**Update:**

```sh
brew upgrade x8ai
x8ai --stop     # ends the old version running in the background, with its shells and agents
x8ai
```

Until you stop it (or quit it with Ctrl-g q), the version already running stays,
and x8ai tells you a newer one is installed.

**Uninstall:**

```sh
x8ai --stop
brew uninstall x8ai
```

To remove your data too, see [Where your data lives](#where-your-data-lives).

---

## Getting started

### 1. Open a space

`x8ai` starts on the **Welcome** screen: a greeting and a command line. Each
folder you work in is a *space*. Type a command:

| Command | What it does |
| --- | --- |
| `/cd <folder>` | Opens a space: a recent one by its name or path (`/cd gym`), or any folder (`/cd ~/code/app`, `/cd ../app`, relative to where you ran `x8ai`). Tab completes folders. |
| `/new <name>` | Makes a new, empty space in `~/Workspaces/<name>` and opens it. |
| `/home` | The workspace with no folder, with a shell in your home folder. |

Typing `/` lists every command. On an empty line, ↑↓ and Enter open one of your
recent spaces, and Esc goes back to the space that is open. `x8ai <folder>`
opens a folder right away.

### 2. Get around a space

A space opens with your shell in its folder. Every key goes to the program in
the focused pane, except **Ctrl-g**: press it, then a key.

- **Ctrl-g t** opens a tab, **Ctrl-g |** and **Ctrl-g -** split the pane, and
  **Ctrl-g ←→↑↓** move between panes.
- **Ctrl-g f** shows the folder's files: Enter opens one in your editor, in a
  tab of its own.
- **Ctrl-g h** shows the Welcome screen while the space keeps running; `/cd`
  back finds it as you left it.
- **Ctrl-g ?** lists every key. They are all in [Keys](#keys) below.

The mouse works too: click a pane or a tab, drag the line between panes to
resize them, and scroll with the wheel. Dragging over text copies it to the
clipboard (hold Shift in a program that uses the mouse, such as vim).

### 3. Trust the folder and launch an agent

Press **Ctrl-g a** for the Agents panel. It lists Claude Code, OpenCode and
Codex (whichever you have installed) and the space's agent sessions. Choose an
agent and press **Enter**.

The first time, x8ai asks you to **trust the folder**: agents run only in
folders you trust. Trust a folder only if you trust its contents, since a
repository can contain files that try to steer an agent. Then it asks you to
**allow the agent** there, and shows:

- the folder and the exact program that will run,
- the model and provider it will use, if you chose one,
- the MCP servers it gets, with their exact commands,
- the skills attached to the session.

Press **y**. The agent starts in a tab of its own. You're asked again only if
something that runs changes, for example a different program or another
provider. Only you, at your keyboard, can answer these questions: a program in
a pane can't.

### 4. Review what it did

In a Git repository, each session works in its own worktree on a branch named
`agent/<agent>/<date>-…`, so your working tree and your current branch are
never changed. In the Agents panel, on a session:

- **c** shows what it changed: its branch, commits, files and the diff, in a
  pager (`q` closes it).
- **Enter** shows the agent, or runs an ended session again.
- **o** opens a shell in its worktree.
- **s** stops it.
- **d** removes the session and its worktree. It asks first, and says what is
  discarded; if the agent made commits, its branch is kept.

To bring an agent's work into your branch, use Git as usual, for example
`git merge agent/claude-code/…`.

In a folder that isn't a Git repository, one agent works directly in the folder
at a time, and x8ai tells you that the agent isn't isolated.

### 5. Choose a model (optional)

By default an agent uses its own settings. To pick a model in x8ai instead:

1. Press **Ctrl-g m** for Models. On a provider, press **s** and paste its API
   key. It is saved in your macOS Keychain, typed as dots, and never shown
   again. **a** adds a model id the provider serves.
2. Choose a model and press **Enter**, then the agent that should use it: its
   next launch does. (Or, in the Agents panel, press **m** on the agent.)

For **local models**, install and start [Ollama](https://ollama.com) yourself,
then press **r** in Models. x8ai looks for Ollama only when you ask.

### 6. Add MCP servers (optional)

Press **Ctrl-g u** for MCP, then **n**:

- **stdio:** a program on your Mac, with its arguments, for example `npx` and a
  package name.
- **http:** a server at a URL.
- **Variables:** list them by name. `NAME` is a secret, which you then save in
  the Keychain with **s**; `NAME=shell` takes the value from your shell.
- **Attached to:** every session, sessions in this folder, or only when you
  choose it at launch (**l** in MCP, or **u** on an agent in the Agents panel).

A server runs only for an agent session, after you approve it. It gets the
basics every program needs (such as `PATH` and `HOME`) plus the variables you
listed, nothing else, and it stops with the agent.

### 7. Use skills and the Catalog (optional)

Press **Ctrl-g k** for the Catalog: every agent, model, MCP server and skill,
whether it's ready and what it still needs. **/** filters it. **Enter** uses
the item selected: a model, server or skill for an agent's next launch, or the
panel where you set it up.

**n** writes your own skill: a name and instructions, which are plain text and
never a secret. A skill is attached to every new session, to sessions in one
folder, or only when you choose it (**l** on an agent in the Agents panel).

The Catalog installs nothing, runs nothing and doesn't go online.

### 8. Set up a space's terminals with add-ons (optional)

Press **Ctrl-g e** for Add-ons, choose a tool and press **Enter**:

| Group | Add-ons |
| --- | --- |
| Shell | Starship (prompt), Autosuggestions, fzf, zoxide, eza, Syntax highlighting |
| Editor | Neovim, LazyVim |
| Tools | ripgrep, fd, lazygit, bat, Memray |
| Look | JetBrains Mono Nerd Font |

If it isn't on your Mac yet, x8ai shows the exact Homebrew command first
(`brew install starship`), and runs it in a tab you can watch once you press
**y**. It never uses `sudo`.

An add-on is on **only in the space you add it to**. New shells there start
with it; other spaces don't change, and your own `~/.zshrc` is never edited.
(If your `~/.zshrc` already turns a tool on, it stays on everywhere; the list
tells you.) Starship runs Git in the folder, so it turns on once you trust the
folder. **d** takes an add-on out of a space without uninstalling it.

Every space has an id, shown on the Welcome screen (`gymRL ws-k3f9qa`).
`/share <space>` on the Welcome screen, or **s** in Add-ons, gives another space
this one's add-ons.

### 9. Close the terminal; nothing stops

x8ai keeps your spaces, shells and agents in a background process of its own.
Close the terminal window, press **Ctrl-g d**, or type `/detach`, and they keep
running: an agent goes on working. Run `x8ai` again, in this terminal or
another, and everything is as you left it (`x8ai <folder>` also opens that
folder). One terminal shows x8ai at a time: opening it in a second one moves it
there.

**Ctrl-g q** (or `/quit`) quits: it ends every shell and agent, and asks first
while something is running. `x8ai --stop` does the same from outside. x8ai also
ends by itself when nothing is open, and when you log out.

---

## Keys

**On the Welcome screen**

| Command | What it does |
| --- | --- |
| `/cd <folder>` | Open a folder as your space (a recent one by its name). |
| `/new <name>` | A new, empty space in `~/Workspaces`. |
| `/home` | The workspace with no folder open. |
| `/share <space>` | Give the open space's add-ons to another space. |
| `/name <your name>` | How the Welcome greets you. |
| `/detach` | Leave x8ai running in the background. |
| `/quit` | Quit x8ai, ending its shells and agents. So does Ctrl-c on an empty line. |
| Esc | Back to the space that is open. |

**In a space, after Ctrl-g**

| Key | What it does |
| --- | --- |
| `t` | A new tab, with a shell. `n` and `p` go to the next and previous tab, `1`–`9` to that one. |
| `\|` and `-` | Split the pane: a new shell to the right, or below. |
| ←→↑↓ and `o` | The pane beside, or the next one. |
| `z` | The pane alone, or back with the others. |
| `x` | Close the pane. If a program is still running in it, you're asked first. |
| `f` | The file list. ↑↓ move, ←→ close and open folders, Enter opens a file in your editor (`$VISUAL`, `$EDITOR`, or `vi`), Esc goes back. |
| `a` | Agents. |
| `m` | Models. |
| `u` | MCP servers. |
| `k` | The Catalog. |
| `e` | Add-ons. |
| `s` | Scroll back through the output (↑↓, PgUp PgDn, `g` `G`; Esc to go back). |
| `h` | The Welcome screen. The space keeps running. |
| `d` | Detach: x8ai keeps running in the background. |
| `q` | Quit, ending every shell and agent. If something is still running, you're asked first. |
| `?` | Every key. |
| Ctrl-g | Sends Ctrl-g to the program. |

A pane closes when its program ends well (`exit`, or quitting the editor); one
that fails stays so you can read it.

**In the panels** (each shows its keys at the bottom; ↑↓ move, Esc goes back to
the panes)

| Panel | Keys |
| --- | --- |
| Agents | On an agent: Enter launch, `m` model, `u` MCP servers, `l` skills for its next launch, `r` take its approval back. On a session: Enter show or run again, `c` changes, `o` shell in its worktree, `s` stop, `d` remove. Anywhere: `t` trust or stop trusting the folder. |
| Models | `s` save a key, `a` add a model id, `d` delete, `r` look for local models, Enter use a model. |
| MCP | `n` new, Enter change, space on or off, `s` save a secret, `l` for the next launch, `d` remove. |
| Catalog | `/` filter, Enter use or open, `n` new skill, `e` change, `d` remove. |
| Add-ons | Enter add or install, `d` remove, `s` share with a space, `r` check again. |

---

## Safety and privacy

- **Nothing runs without you.** Agents and MCP servers start only in folders you
  trusted, after you approve exactly what will run. A repository can't trust or
  approve anything for itself, and a program in a pane can't answer x8ai's
  questions.
- **Your keys stay in the Keychain.** API keys and MCP secrets are never shown,
  and never written to files, logs or your projects. An agent gets a key only
  for the session you chose it for.
- **MCP servers get only what you list.** Apart from basics such as `PATH`, they
  receive only the variables you listed: never your provider keys or other
  secrets from your shell.
- **No surprise network traffic.** x8ai itself makes no network connections. It
  looks for Ollama on your own Mac only when you ask, and add-ons download only
  through Homebrew, after you agree. Agents talk to their providers themselves.
- **Only you reach the background x8ai.** It listens on a socket in a folder
  only your account can open, answers only your own processes, and opens no
  network port.
- **Your agent settings stay yours.** x8ai never edits `~/.claude`, OpenCode's
  or Codex's global configuration. It gives agents their settings per session
  only.
- **Agents run as you.** Like any program you start in a terminal, an agent can
  read your files and reach the network. x8ai doesn't sandbox agents, so trust
  folders and approve agents deliberately.

Closing a pane, or quitting x8ai, while a program or an agent is running asks
you first.

## Where your data lives

| What | Where |
| --- | --- |
| Recent and trusted folders, approvals, providers, MCP servers, skills, spaces and their add-ons (no secrets) | `~/Library/Application Support/com.x8ai.workspace/` |
| Each space's shell setup for its add-ons, rewritten for every new shell | `~/Library/Application Support/com.x8ai.workspace/spaces/<id>/zsh/` |
| LazyVim, when added | `~/.config/x8ai-lazyvim/` (your `~/.config/nvim` is untouched) |
| API keys and MCP secrets | macOS Keychain, under `com.x8ai.workspace.providers` and `com.x8ai.workspace.mcp` |
| Agent worktrees | `~/.x8ai/worktrees/` |
| The background x8ai: its socket, lock, process id and log (yours alone) | `~/.x8ai/server/` |
| MCP sockets of running agent sessions | `~/.x8ai/mcp-<process id>/` |

To start over: run `x8ai --stop`, delete
`~/Library/Application Support/com.x8ai.workspace/` and `~/.x8ai/`, and remove
the Keychain items in Keychain Access. Deleting `~/.x8ai/` also deletes agents'
worktrees; merge any work you want to keep first.

## Troubleshooting

- **An agent shows "not installed".** Check that its command works in a new
  terminal window. x8ai gives agents the environment of the terminal it was
  started from; if you changed your `PATH`, run `x8ai --stop` and start `x8ai`
  again from a new terminal.
- **I can't choose a model for my agent.** Save a key for a provider that agent
  supports (see the table above), or add a model id in Models.
- **An MCP server isn't ready.** Save its secrets in MCP (**s**), and check that
  its command is installed.
- **An add-on won't install.** It needs Homebrew (brew.sh).
- **macOS asks whether x8ai may use a key in your Keychain.** Choose Always
  Allow. It may ask again after an update.
- **"Make the window wider to show the sidebar."** Panels need a terminal at
  least 60 columns wide.
- **"x8ai … is installed; this is …, started before."** You updated x8ai while
  it was running. Quit it (Ctrl-g q) or run `x8ai --stop`, then run `x8ai`.
- **"The background x8ai stopped unexpectedly."** What it said is in
  `~/.x8ai/server/log`. Please open an issue with it.
- **"This terminal is inside x8ai already."** You ran `x8ai` in one of its own
  shells. Press Ctrl-g h for its Welcome screen instead.

---

## For contributors

```sh
cargo run -p x8ai         # run x8ai from source, in this terminal
cargo test -p x8ai        # its tests, including end-to-end tests on a PTY
pnpm check                # everything CI checks: types, tests, formatting, lints
```

`cargo run -p x8ai` starts a background x8ai from your build. Run
`cargo run -p x8ai -- --stop` before switching back to the installed one.

To release `x8ai`, bump the version in `crates/tui/Cargo.toml` and push a tag
`v<version>`. CI builds a universal binary (`scripts/release/build-x8ai.sh`),
publishes it as a GitHub release, and updates the formula in
`aashirvad08/homebrew-tap` when the `HOMEBREW_TAP_TOKEN` secret is set
(`.github/workflows/release.yml`).

```
crates/tui/  x8ai: the Welcome screen, spaces, panes and panels, the background
             process and the terminal that attaches to it
crates/      the Rust core: terminals, workspace files, agents, Git, Keychain,
             providers, MCP, skills, the catalog and add-ons
src-tauri/   a desktop app on the same core (Tauri), built from source with
src/         `pnpm install && pnpm tauri build`; it shares x8ai's data
docs/        design documents and decision records
```

To learn how it works, read [Architecture](docs/architecture.md),
[Security](docs/security.md), [Agents](docs/agent-runtime.md),
[Agent sessions and worktrees](docs/multi-agent.md), [Models](docs/models.md),
[MCP servers](docs/mcp.md), [Catalog and skills](docs/catalog.md) and the
[decision records](docs/decisions/) (x8ai's start at
[0020](docs/decisions/0020-terminal-version.md)).

**Ground rules**

1. Keep parts separate: rules belong in the crates, not in what draws them.
2. Prefer simple designs, and justify every new dependency.
3. Keep commits small and buildable, and never hide errors.
4. No mock implementations posing as real features.
5. Never hard-code credentials, and never tie x8ai to one agent, provider or
   MCP server.
6. Changes to who can start programs, or reach x8ai's socket, are security
   changes. Review them as such.
