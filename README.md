# x8ai Workspace

**A terminal-first workspace for coding with AI agents, on macOS.**

x8ai Workspace puts your terminals, your code and the coding agents you already
use in one window. Open a project, pick an agent such as Claude Code, OpenCode or
Codex, choose the model it should use, and let it work. Each agent gets its own
copy of your repository, so it never touches your checkout while it works, and
you review what it changed.

The app doesn't come with its own AI. It runs the agents and models you choose,
and it doesn't favor any vendor.

---

## What you can do

- **Or stay in your terminal.** `brew install aashirvad08/tap/x8ai`, then run
  `x8ai`: the same Welcome screen and spaces, with their shells, in the terminal
  you already use (see [x8ai in your terminal](#x8ai-in-your-terminal)).
- **Start from the Welcome screen.** Type `/cd` to open a folder as your space
  and `/home` for the workspace with no folder; ⇧⌘H brings it back any time.
- **Work in real terminals.** Full terminals with tabs and split panes, running
  your own shell.
- **Browse, search and edit your code.** A file explorer, search across the
  folder, quick open (⌘P) and an editor with syntax highlighting.
- **Run coding agents safely.** Agents run only in folders you trust, and only
  after you approve the exact program and settings. In a Git repository each
  agent session works in a Git worktree of its own, on its own branch.
- **Run several agents at once** and see what each one changed, file by file,
  before you merge anything.
- **Choose the model.** Save an API key once for Anthropic, OpenAI, Google or
  OpenRouter, or use local models with Ollama, and pick a model for each session.
- **Give agents tools with MCP servers.** Add a server once and attach it to the
  sessions that need it.
- **Attach skills.** Skills are reusable instructions, such as "write a failing
  test first", that you add to a session.
- **Browse everything in one place.** The Catalog lists every agent, model, MCP
  server and skill, whether it's ready, and what it still needs.

## Supported agents and providers

| Agent | Choose its model in the app | MCP servers | Skills |
| --- | --- | --- | --- |
| [Claude Code](https://docs.anthropic.com/en/docs/claude-code) | Anthropic, OpenRouter, Ollama | ✓ | ✓ |
| [OpenCode](https://opencode.ai) | Anthropic, OpenAI, Google, OpenRouter, Ollama | ✓ | — |
| [Codex](https://github.com/openai/codex) | OpenAI | — | — |

Any agent can also use its own configuration, exactly as it does in your
terminal. You install the agents yourself; the app finds them on your `PATH`.
Any other command-line tool runs in the app's terminals as usual.

---

## Install

### In your terminal

```sh
brew install aashirvad08/tap/x8ai
x8ai
```

`x8ai` runs on macOS 13 or later, on Apple silicon and Intel. It is the
workspace as a full-screen program in the terminal you already use (see
[x8ai in your terminal](#x8ai-in-your-terminal)). Agents, models, MCP servers,
skills and add-ons are in the app for now, and come to `x8ai` step by step.

### The app

There is no downloadable release of the app yet, so you build it from source.
The build takes a few minutes the first time.

**You need:**

- macOS 13 or later
- Xcode Command Line Tools: `xcode-select --install`. These include Git.
- [Node.js](https://nodejs.org) 22.12 or later
- [Rust](https://rustup.rs), installed with rustup. The right version is picked
  up automatically.

**Build it:**

```sh
git clone https://github.com/aashirvad08/x8ai-workspace.git
cd x8ai-workspace
corepack enable          # provides pnpm
pnpm install
pnpm tauri build
```

The app is created at `target/release/bundle/macos/x8ai Workspace.app`. Drag it
into your Applications folder, or open it right away:

```sh
open "target/release/bundle/macos/x8ai Workspace.app"
```

**Install an agent** if you don't have one yet, for example Claude Code, OpenCode
or Codex. Follow each agent's own instructions. The app doesn't install them.

---

## x8ai in your terminal

`x8ai` starts on the Welcome screen. Type a command:

| Command | What it does |
| --- | --- |
| `/cd <folder>` | Opens a space: a recent one by its name or path (`/cd gym`), or any folder (`/cd ~/code/app`, `/cd ../app`; relative to where you started `x8ai`). Tab completes folders. |
| `/new <name>` | Makes a new, empty space in `~/Workspaces/<name>` and opens it. |
| `/home` | The workspace with no folder, with a shell in your home folder. |
| `/name <your name>` | How the Welcome greets you. |
| `/quit` | Leaves `x8ai`. So does Ctrl-c on an empty line. |

On an empty line, ↑↓ and Enter open one of your recent spaces, and Esc goes
back to the space that is open. `x8ai <folder>` opens a folder right away.

A space shows its shell. Every key goes to the shell except **Ctrl-g**, after
which:

| Key | What it does |
| --- | --- |
| `h` | The Welcome screen. The space keeps running, and `/cd` back finds its shell as you left it. |
| `s` | Scroll back through the shell's output (↑↓, PgUp PgDn, `g` `G`; Esc to go back). |
| `q` | Quit. If a program is still running, you're asked first. |
| Ctrl-g | Sends Ctrl-g to the shell. |

`x8ai` shares the app's recent spaces, space ids and trust, so a folder opened
in one is recent in the other.

## Getting started

### 1. Open a space from the Welcome screen

The app starts on the **Welcome** screen, its head: a greeting and a command
line. Each folder you work in is a *space*, and one space is open at a time.

| Command | What it does |
| --- | --- |
| `/cd <folder>` | Opens a space. A recent one opens at once, by its path or the start of its name (`/cd gymRL`, `/cd gym`, `/cd ~/code/app`). Any other folder opens the macOS folder picker at that place, where you choose it. `/cd` alone opens the picker. |
| `/new <name>` | Makes a new, empty space in `~/Workspaces/<name>` and opens it (see step 9). |
| `/share <space>` | Gives another space this space's add-ons, by its name or id (see step 9). |
| `/home` | The workspace with no folder open, and a new terminal in your home folder. |
| `/name <your name>` | How the Welcome greets you. `/name` alone goes back to your Mac account's name. |
| `/get [agent]` | Gives an agent what the other sessions did (see step 8). |
| `/give [agent]` | Passes an agent's work to another session (see step 8). |

Typing `/` lists the commands; after `/cd `, your recent spaces (Tab completes,
↑↓ choose). **Esc** goes to the space as it is, and **⇧⌘H** (or **⌂** in the
status bar) brings the Welcome back while everything keeps running. The
app reopens your last space behind the Welcome when it starts. **⌘O** and
**⌃R** open folders from the workspace too.

### 2. Trust the folder

Agents run only in folders you trust. The status bar shows **Untrusted** until
you trust the folder. Open **Agents** (**⇧⌘A**) and click **Trust Folder…**, then
confirm. Trust a folder only if you trust its contents: a repository can contain
files that try to steer an agent.

### 3. Launch an agent

In **Agents**, find your agent's card, for example Claude Code, and click
**Launch**. Before anything runs, a dialog shows:

- the folder and the exact program that will run,
- the model provider it will talk to, if you chose one,
- the MCP servers it gets, with their exact commands,
- the skills attached to the session.

Click **Allow**. The agent starts in a terminal tab of its own. You're asked
again only if something that runs changes, for example a different program or
another provider.

### 4. Review what it did

In a Git repository, each session works in its own worktree on a branch named
`agent/<agent>/<date>-…`, so your working tree and your current branch are never
changed. In the session's card:

- **Changes** lists the changed files and commits; click a file to read it.
- **Show** brings back the agent's terminal, and **Start** runs an ended
  session again. **Stop** and **Restart** appear while it runs.
- **Remove** deletes the session's worktree. It asks first if there is
  uncommitted work. If the agent made commits, its branch is kept.

To bring an agent's work into your branch, use Git as usual, for example
`git merge agent/claude-code/…`.

In a folder that isn't a Git repository, one agent works directly in the folder at
a time, and the app tells you that the agent isn't isolated.

### 5. Choose a model (optional)

By default an agent uses its own settings. To pick a model in the app instead:

1. Open **Models** (**⇧⌘M**) and paste an API key for a provider. It is saved in
   your macOS Keychain, and the app never shows it again.
2. Back in **Agents**, choose the model in the agent's card before you click
   **Launch**.

For **local models**, install and start [Ollama](https://ollama.com) yourself,
then click refresh in Models. The app looks for Ollama only when you ask.

### 6. Add MCP servers (optional)

Open **MCP** (**⇧⌘U**) and click **+**:

- **stdio:** a program on your Mac, with its arguments, for example `npx` and a
  package name.
- **HTTP:** a server at a URL.
- **Variables:** list them by name. Each value is either a secret, saved in the
  Keychain, or taken from your shell.
- **Scope:** every new session, sessions in this folder, or only when you choose
  it at launch.

A server runs only for an agent session, after you approve it in the launch
dialog. It gets the basics every program needs (such as `PATH` and `HOME`) plus
the variables you listed, nothing else, and it stops with the agent.

### 7. Use skills and the Catalog (optional)

Open **Catalog** (**⇧⌘K**) to see everything in one place. Search it, filter it
by category (Agents, Models, MCP, Skills) or by status, and open an item to see
its details and what it needs.

- **Drag a model onto the terminal** (or click **Open in a terminal**) to open it
  right away, in the agent that can use it: an OpenAI model in Codex, an
  Anthropic one in Claude Code. The launch dialog appears as usual. A model can
  be dragged once its provider's key is saved.
- **Use for the next launch** and **Attach to the next launch** fill in the
  agent's card for you. Nothing starts until you click **Launch**.
- **+** in the Catalog's header creates your own skill: a name and
  instructions, which are plain text and never a secret. Skills are attached to
  every new session, to sessions in one folder, or only when you choose them.

The Catalog installs nothing, runs nothing and doesn't go online. It shows what
the app knows and takes you to the right place to set things up.

### 8. Hand work from one agent to another (optional)

Switching agents mid-task, for example from Claude Code to Codex? Type `/get
codex` on the Welcome screen, or click **Get context…** on Codex's session card.
Choose which sessions it comes from and what to include: what changed (files and
line counts), the diff, the last lines of their terminals, and a note. You see
the exact text before anything is sent. **Put in its input** pastes it into
Codex's input; you read it there and press Enter. `/give` (or **Give context…**)
works the other way round, from one session to another.

Everything in the text goes to the receiving agent and its model provider, and
terminal output can contain secrets, so read it first. The app never reads an
agent's own history files.

### 9. Set up a space's terminal with add-ons (optional)

Open **Add-ons** (**⇧⌘X**) and click **Add** (or **Install**) next to a tool:

| Group | Add-ons |
| --- | --- |
| Shell | Starship (prompt), Autosuggestions, fzf, zoxide, eza, Syntax highlighting |
| Editor | Neovim, LazyVim |
| Tools | ripgrep, fd, lazygit, bat, Memray |
| Look | JetBrains Mono Nerd Font |

If it isn't on your Mac yet, a dialog shows the exact Homebrew commands first
(`brew install starship`), and the install runs in a terminal tab you can watch.
Homebrew must be installed (brew.sh); the app never uses `sudo`.

An add-on is on **only in the space you add it to**. Its new terminals start with
it; other spaces don't change, and your own `~/.zshrc` is never edited. (If your
`~/.zshrc` already turns a tool on, it stays on everywhere; the list tells you.)
Programs such as ripgrep work in every terminal once installed. Starship runs Git
in the folder, so it turns on once you trust the folder. **Remove** takes an
add-on out of a space without uninstalling it.

Every space has an id, shown on the Welcome screen (`gymRL ws-k3f9qa`):

- `/share <space>` gives another space this one's add-ons. Press Enter to give
  all of them, or press ↓, then Space to uncheck some first.
- `/new <name>` makes a new, empty space in `~/Workspaces/<name>`, with no
  add-ons.

---

## Keyboard shortcuts

| | |
| --- | --- |
| **Welcome** | ⇧⌘H Show Welcome · on it: `/cd <folder>` · `/new <name>` · `/share <space>` · `/home` · `/get [agent]` · `/give [agent]` · `/name <your name>` · Esc back to the space |
| **Folders and files** | ⌘O Open Folder · ⌃R Open Recent · ⌘P Go to File · ⌘N New File · ⌘S Save · ⌥⌘S Save All · ⌘W Close Editor · ⌘F Find in File |
| **Panels** | ⇧⌘E Files · ⇧⌘F Search in Folder · ⇧⌘A Agents · ⇧⌘M Models · ⇧⌘U MCP · ⇧⌘K Catalog · ⇧⌘X Add-ons · ⌘B Toggle Sidebar |
| **Terminal** | ⌃\` Show or Hide Terminal · ⌃⇧\` New Terminal · ⌘D Split Right · ⇧⌘D Split Down · ⌘] / ⌘[ Next / Previous Pane · ⌘W Close Pane |
| **Explorer** | Arrow keys and Enter to move and open · F2 Rename · ⌘⌫ Move to Trash |
| **Everything else** | ⇧⌘P All Commands |

---

## Safety and privacy

- **Nothing runs without you.** Agents and MCP servers start only in folders you
  trusted, after you approve exactly what will run. A repository can't trust or
  approve anything for itself.
- **Your keys stay in the Keychain.** API keys and MCP secrets are never written
  to files, logs or your projects. An agent gets a key only for the session you
  chose it for.
- **MCP servers get only what you list.** Apart from basics such as `PATH`, they
  receive only the variables you listed: never your provider keys or other
  secrets from your shell.
- **No surprise network traffic.** The app makes no network connections when it
  starts. It contacts Ollama on your own Mac only when you ask. Agents talk to
  their providers themselves.
- **Your agent settings stay yours.** The app never edits `~/.claude`, OpenCode's
  or Codex's global configuration. It gives agents their settings per session
  only.
- **Agents run as you.** Like any program you start in a terminal, an agent can
  read your files and reach the network. The app doesn't sandbox agents, so trust
  folders and approve agents deliberately.

Closing a terminal, or quitting the app while a program or an agent is running,
asks you first.

## Where your data lives

| What | Where |
| --- | --- |
| Recent and trusted folders, approvals, providers, MCP servers, skills, spaces and their add-ons (no secrets) | `~/Library/Application Support/com.x8ai.workspace/` |
| Each space's terminal setup for its add-ons, rewritten for every new terminal | `~/Library/Application Support/com.x8ai.workspace/spaces/<id>/zsh/` |
| LazyVim, when added | `~/.config/x8ai-lazyvim/` (your `~/.config/nvim` is untouched) |
| API keys and MCP secrets | macOS Keychain, under `com.x8ai.workspace.providers` and `com.x8ai.workspace.mcp` |
| Agent worktrees | `~/.x8ai/worktrees/` |

To start over, quit the app, delete that Application Support folder, and remove
the Keychain items in Keychain Access.

## Troubleshooting

- **An agent says "Not installed".** Install it, and check that its command
  works in a new terminal window. The app uses the `PATH` from your login shell.
  Then click refresh in Agents.
- **Launch is disabled.** Open a folder first, then trust it.
- **Ollama shows "Not found" or "not running".** Install Ollama or start it, then
  click refresh in Models.
- **I can't choose a model for my agent.** Save a key for a provider that agent
  supports (see the table above), or add a model id in Models.
- **An MCP server shows as not configured.** Save its secret variables in MCP,
  and check that its command is installed.

---

## For contributors

```sh
pnpm tauri dev      # run the app with hot reload
cargo run -p x8ai   # run x8ai, the terminal version, in this terminal
pnpm check          # everything CI checks: types, tests, formatting, lints
pnpm test           # frontend tests only
cargo test --workspace
```

`pnpm tauri build --bundles dmg` also builds a DMG. It drives Finder, so your
terminal needs permission to control Finder (System Settings → Privacy &
Security → Automation).

To release `x8ai`, bump the version in `Cargo.toml` and push a tag `v<version>`.
CI builds a universal binary (`scripts/release/build-x8ai.sh`), publishes it as
a GitHub release, and updates the formula in `aashirvad08/homebrew-tap` when
the `HOMEBREW_TAP_TOKEN` secret is set (`.github/workflows/release.yml`).

The app is a [Tauri](https://tauri.app) app with a Rust core and a React +
TypeScript interface:

```
crates/      the Rust core: terminals, workspace files, agents, Git, Keychain,
             providers, MCP, skills and the catalog (none of it depends on Tauri)
crates/tui/  x8ai, the terminal version: the same core in a full-screen program
src-tauri/   the desktop app: window, native commands and their permissions
src/         the interface (React + TypeScript)
docs/        design documents and decision records
```

To learn how it works, read [Architecture](docs/architecture.md),
[Security](docs/security.md), [Agents](docs/agent-runtime.md),
[Agent sessions and worktrees](docs/multi-agent.md), [Models](docs/models.md),
[MCP servers](docs/mcp.md), [Catalog and skills](docs/catalog.md) and the
[decision records](docs/decisions/).

**Ground rules**

1. Keep parts separate: no business logic in UI components.
2. Prefer simple designs, and justify every new dependency.
3. Keep commits small and buildable, and never hide errors.
4. No mock implementations posing as real features.
5. Never hard-code credentials, and never tie the app to one agent, provider or
   MCP server.
6. Changes to `src-tauri/capabilities/` are security changes. Review them as
   such.
