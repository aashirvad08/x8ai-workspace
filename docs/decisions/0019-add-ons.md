# 0019. Add-ons: tools for one space's terminals, installed when the user adds them

**Status:** Accepted. Amended (see the end): a program for the whole Mac can be turned off in one space.

## Context

People set their terminal up with the same handful of tools: a prompt
(Starship), syntax highlighting and autosuggestions for zsh, a fuzzy finder, an
editor setup (Neovim with LazyVim), search tools (ripgrep, fd), a font with
icons. Doing it by hand means finding each one, running `brew install`, and
editing `~/.zshrc`, which then changes every terminal on the Mac.

The workspace hosts terminals and knows which space (folder) each belongs to.
It could offer these tools, on in the spaces where the user wants them and not
elsewhere. Until now the app installed nothing (ADR 0018: the catalog cannot
install). The questions:

1. Who **installs**, what does it run, and how does the user consent?
2. How is a tool on in **one space** only, without editing the user's files?
3. How are spaces **named**, so one space's set can be given to another?
4. What may the **webview** decide?

## Decision

**Add-ons are a closed list in the app (`crates/addons`).** Each entry says what
it needs on the Mac (a program on the login `PATH`, a file under Homebrew's
prefix, a font, a Neovim configuration), how it is installed (a Homebrew
formula or cask, or a `git clone` into `~/.config/<name>`), and what it adds to a
space's zsh. Nothing outside the app can add an entry or change what one runs:
the webview, the store file and the folder name add-ons by id only.

**Installing is the user's, seen and confirmed.** Adding an add-on whose
program is missing shows a native dialog with the exact commands
(`brew install starship`), which the webview cannot answer. Only then does the
native side record a one-time token, and the install runs once in a terminal
tab the user watches (`/bin/sh -c` over a script where every word is quoted;
each command is printed before it runs; the first failure stops it). Nothing
runs with `sudo`, nothing is piped from the network into a shell, and Homebrew
is not installed by the app: without it, the view says to install it from
brew.sh. When the install ends well and everything is found, the add-on is added
to the space it was confirmed for. The catalog is unchanged (ADR 0018): it
still cannot install.

**On in one space, through `ZDOTDIR`.** A space's terminal is the user's login
zsh with `ZDOTDIR` set to a folder of the space's own in the app's data
directory. zsh reads its startup files from there; each first reads the user's
own file from where zsh would have found it, and the last hands `ZDOTDIR` back,
so a zsh started inside reads only the user's files. After the user's
`.zshrc`, the space's turns on its add-ons, each guarded so it does nothing if
its program is gone or it is already on. The history file stays the user's.
Other variables (`NVIM_APPNAME` for LazyVim, whose configuration is its own and
leaves `~/.config/nvim` alone) are set the same way. The user's files are never
written. Terminals of other spaces, and of other apps, are unchanged. The font
add-on sets the terminal font while its space is open.

Some add-ons are programs (ripgrep, fd, Neovim): once installed they are on the
Mac's `PATH` in every terminal, which an app cannot scope. The list says so; a
space keeps them in its set so that sharing carries them.

**Spaces have ids.** Every folder opened as a workspace, and the workspace with
no folder, gets an id (`ws-` and six letters or digits) the first time it is
opened, kept in `spaces.json` with its add-on ids. `/share <space>` on the
welcome screen gives another space add-ons of the open one, all of them unless
the user unchecks some; the native side accepts only add-ons the open space
has. `/new <name>` makes the empty folder `~/Workspaces/<name>` and opens it: the
webview names a folder, never a path, and an existing folder is never opened
this way (ADR 0009).

**Trust still decides what runs in a folder.** Starship runs `git` in the
folder, and a repository's configuration can make `git` run commands, so it is
on only in a trusted folder (the space with no folder is the user's home). An
untrusted folder's terminal starts without it, as before.

## Consequences

- One click and a confirmation set up a tool, and the user sees exactly what
  runs. A space looks the way it was set up, and another space is unaffected.
- Lines in the user's own `~/.zshrc` still apply everywhere. The view says when
  the user's files already turn an add-on on, since removing the line is what
  makes it per space.
- Shell add-ons need zsh, the macOS default. Other shells get program add-ons
  and the font only.
- Removing an add-on from a space uninstalls nothing: other spaces may use it.
- New add-ons are a code change, reviewed like any other: an entry in
  `crates/addons/src/registry.rs` and its tests.

## Amendment (2026-10-28): programs turned off in one space

The Decision says programs on the Mac's `PATH` cannot be scoped. They can be
hidden from one space's terminals, which is what a user turning Neovim "off"
in a space expects. A space keeps a list of the programs turned off in it
(`off` in `spaces.json`, beside its add-ons). For each, x8ai writes a stand-in
to the space's own folder (`spaces/<id>/off/<program>`, the user's alone) that
says the program is off there and exits 127, as a missing program does. The
space's zsh files put that folder first on the `PATH` after the user's
`.zshrc`, and again after their `.zlogin`, so the user's own `PATH` lines
(Homebrew's `shellenv`, `path_helper` in `/etc/zprofile`) cannot put the real
program back in front; the folder is on the `PATH` once. Programs started from
such a shell find the stand-in too (Git's editor), and so does the editor the
file list opens in `x8ai`. Only new shells change; other spaces' terminals,
and other apps', never do.

It is an off switch, not a lock: the program's full path still runs it, and
so does anything that does not look it up on the `PATH`. Hiding it for real
would take a sandbox around every shell. It needs zsh, as shell add-ons do.
Agents keep the environment `x8ai` started with (ADR 0022). `/share` does not
carry what is off: it is about one space. The app's terminals use the same
code, so they honour the list; its Add-ons view does not show it yet, and an
app built before this drops the list when it next writes the file.
