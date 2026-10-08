# 0020. `x8ai`: the workspace in the terminal, installed with Homebrew

**Status:** Accepted

## Context

The workspace calls itself terminal-first, yet it ships only as a Mac app that
people build from source. People want two things it does not give them:

1. **One command to install it**, as with any developer tool:
   `brew install …`.
2. **The workspace in the terminal they already use**, not in a window: start it,
   get the Welcome screen, open a space, and work in that space's shell.

A Homebrew cask can install the app, but Homebrew now refuses apps that fail
Gatekeeper, which needs a paid Developer ID, signing and notarization (Phase
12). And it would still open a window. A command-line program installed by a
Homebrew formula needs none of that, and runs where the user already is.

The Tauri-free crates were kept that way for "a future CLI" (ADR 0002). The
questions:

1. Where does the terminal version live, and what does it reuse?
2. How does it draw, and how do real shells run inside it?
3. What do keys mean, when ⌘ never reaches a terminal program?
4. Does it share anything with the app?
5. How is it distributed?

## Decision

**A second host, `crates/tui`: the `x8ai` package and command.** Like
`src-tauri`, it is a host that wires crates together; unlike it, it has no
webview and no Tauri. It reuses `x8ai-pty` for every shell (login shell, flow
control, hangup on close), `x8ai-workspace` for spaces, recent folders, ids and
trust, and `x8ai-core`. Logic the app kept in TypeScript (the Welcome's
commands and matching, `src/home/home.ts`) is written again in Rust, with the
same rules and tests; rules both hosts need move into the crates
(`new_space_name`, `NEW_SPACES_FOLDER`, `account_name`).

**Drawing: ratatui over crossterm.** A full-screen program in the alternate
screen, raw mode, with bracketed paste. Colors are the terminal's own, plus the
app's raspberry and indigo (24-bit where `COLORTERM` says so, the nearest of 256
otherwise), so the user's light or dark theme is kept.

**Shells in panes: `alacritty_terminal` as the emulator.** Each pane's PTY
output is parsed into a grid, which is drawn as cells; nothing a program prints
reaches the user's terminal as it is. The emulator answers the program's
queries (cursor position, device attributes), which programs such as crossterm
apps need to start. OSC 52 is off and the kitty keyboard protocol is not
offered (keys are encoded the xterm way, `keys.rs`). Synchronized updates (DEC
2026) are honored with the emulator's timeout.

**Keys: a prefix, Ctrl-g.** In a space every key goes to the shell except
Ctrl-g, after which `h` shows the Welcome, `s` scrolls back, `q` quits, and
Ctrl-g sends Ctrl-g. Not Ctrl-Space, which macOS keeps for input sources, nor
Ctrl-b (tmux) or Ctrl-a (readline and screen). On the Welcome screen, Esc goes
back to the space and Ctrl-c on an empty line quits.

**The same spaces as the app.** `x8ai` reads and writes the app's own stores in
its data folder (`~/Library/Application Support/com.x8ai.workspace`), so recent
spaces, space ids and trust are shared. Each change reads the store first and
writes it straight back, so the two running together lose as little as
possible. One space is shown at a time, as in the app, and the last one opens
behind the Welcome. Each space opened in a run keeps its shell until `x8ai`
quits, so `/cd` back finds it as it was left.

**A typed folder opens.** In the app, `/cd <folder>` only says where the native
picker starts, because the webview is untrusted (ADR 0009). Here there is no
webview: the command line is the user, as in `cd`. A relative path is relative
to where `x8ai` was started; `x8ai <folder>` opens one at once. Opening a
folder does not trust it; trust stays an explicit, separate act.

**Terminals of its own.** Variables that point at the terminal `x8ai` runs in
(`TMUX`, `KITTY_WINDOW_ID`, `TERM_SESSION_ID`, …, `x8ai_pty::HOST_TERMINAL`)
are removed from every session, in both hosts, as Claude Code's child-session
marker already was: `tmux` inside a pane then starts, and no tool draws for a
terminal it is not in.

**Distribution: a universal binary on GitHub Releases, and a Homebrew tap.** A
tag `v<version>` builds `x8ai` for Apple silicon and Intel, joins them with
`lipo`, signs the result ad hoc (Apple silicon runs only signed code), and
publishes it as a release (`.github/workflows/release.yml`,
`scripts/release/`). The formula in `<owner>/homebrew-tap` points at it, so
`brew install aashirvad08/tap/x8ai` installs it. Files a formula installs are
not quarantined, so neither notarization nor a Developer ID is needed.

## Consequences

- The workspace installs with one command and runs in any terminal, with no
  Apple account, while the app keeps working as it was.
- Step 1 shows a space with one shell. Panes and tabs, the file list, agents,
  models, MCP, skills, the catalog and add-ons follow in later steps
  (`docs/roadmap.md`). Until add-ons come, a space's shell in `x8ai` starts
  without them, with only `X8AI_SPACE`.
- Closing the terminal window ends `x8ai` and its shells, as closing any
  terminal does. Keeping spaces running without a window needs a background
  process to reattach to, a later step.
- Two hosts mean two places to wire each feature. The crates keep the rules;
  the hosts stay thin, and a rule needed by both moves into a crate.
- Programs in a pane see `TERM=xterm-256color` and an emulator as capable as
  Alacritty's; they do not get the colors or pixel size of the user's terminal,
  which are not known here.

## Alternatives considered

- **A Homebrew cask for the Mac app:** needs Developer ID signing and
  notarization first (Phase 12), and is still a window, not the terminal.
- **The `vt100` crate as the emulator:** simpler, but it parses only: it does
  not answer cursor-position or device-attribute queries, and programs that ask
  (crossterm-based tools, some prompts) stall or fail.
- **tmux or zellij for the panes:** less code, but a dependency the user must
  install, and the workspace would not own its keys, screen or sessions.
- **cargo-dist for releases:** it generates the release workflow and the
  formula, but as a large generated workflow tied to its own version. One macOS
  binary and one formula fit in a short workflow and two scripts that can be
  read and run locally.
