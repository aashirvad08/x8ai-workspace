# 0024. `x8ai` in the background

**Status:** Accepted

## Context

Until now `x8ai` was one process in the user's terminal (ADR 0020 to 0023).
Closing that terminal hung up every shell and agent in every space. Agents
work for a long time, and the user wants to close the window, or their
laptop's lid, and come back later, maybe from another terminal, to find them
still working, as tmux does for shells. The questions:

1. What holds the spaces, their shells and agents, when no terminal is open?
2. How does `x8ai` in a terminal reach it, and who else can?
3. How is the screen drawn in a terminal that is not where the programs run?
4. What if `x8ai` is opened in two terminals?
5. How does it end, and what happens on an update?
6. Which environment do shells and agents get?

## Decision

**A background `x8ai` holds everything; `x8ai` in a terminal attaches to
it.** The background process is the same binary, started with `--server` by
the first `x8ai` that finds none. It calls `setsid` first, so it is in a
session of its own and no terminal's hangup reaches it. Its input and output
are `/dev/null` and its errors go to `~/.x8ai/server/log`. Its folder is the
home folder. It runs what the single process ran: the Welcome, the spaces,
every pane's emulator, agents, the MCP runtime and the bridge sockets.

`x8ai` in a terminal is now a client. It puts the terminal in raw mode, on the
alternate screen, with the mouse and bracketed paste. It says hello (protocol,
version, its folder, `x8ai <folder>`, the terminal's size, whether it shows
24-bit color), then sends what crossterm reads: keys, the mouse, pastes and
size changes. It writes what comes back to the terminal unchanged, until told
to end. Then it gives the terminal back and prints why.

**The background `x8ai` draws for the terminal attached.** ratatui draws into
bytes at that terminal's size and colors, and only the changes are sent, as
before. A terminal that attaches or resizes gets the whole screen. Drawing
goes through a queue of 64 updates. When a terminal reads too slowly to keep
up, updates are dropped, never the programs' output, and the screen is drawn
whole once the terminal catches up. A program's output never waits for a
terminal.

**One terminal at a time.** The last `x8ai` opened takes it over. The one
before is told "x8ai is open in another terminal now" and goes back to its
shell. Two terminals of different sizes cannot both be drawn exactly, and
whose keys win would be a question.

**Leaving and ending.**
- Ctrl-g d, `/detach` on the Welcome, or closing the terminal leaves
  everything running. The next `x8ai`, in any of the user's terminals on this
  Mac, finds it as it was left, and `x8ai <folder>` also opens that folder.
- Quitting (Ctrl-g q, `/quit`) ends everything, as before, and still asks
  first while something runs. The question now says that Ctrl-g d leaves it
  running instead. Every program has ended before the terminal is given back.
- `x8ai --stop` ends it from outside: SIGTERM to the process that holds the
  lock, which ends as quitting does. It does not depend on the version.
- With no space open and no terminal attached for 10 s, it ends: there is
  nothing to keep. Detaching with nothing open ends it at once.

**Only the user reaches it.**
- Its folder `~/.x8ai/server/` is the user's alone (0700) and is refused if it
  belongs to anyone else.
- It listens only on a socket there (0600). Each connection is checked with
  `getpeereid`, and only the user's own processes are answered.
- A lock (`flock`) on `lock` makes it one per user. The process id beside the
  lock is what `--stop` signals.
- There is no network.

**Versions say so.** The hello carries a protocol number. A background `x8ai`
of another protocol refuses the terminal and tells it how to go on: `x8ai
--stop`, then `x8ai`. One of the same protocol but another version attaches,
and says that the newer version starts after quitting. Frames are a kind, a
length and the bytes; everything but drawing is JSON, so the hello stays
readable across versions.

**Programs get the environment of the `x8ai` that started it.** That is the
user's terminal's environment, as ADR 0022 decided. It is now the one from
when the background `x8ai` started, not from each terminal that attaches.
Login shells still read the user's startup files, so a new shell sees a
changed `.zshrc`. Agents get the environment the background `x8ai` started
with, until it restarts. The background `x8ai` also sets `X8AI_INSIDE`, which
every program it starts inherits. `x8ai` refuses to run where that is set,
since in one of its own panes it would attach to itself.

## Consequences

- Closing the terminal, a crash of the terminal app, or losing an SSH
  connection to the Mac no longer stops shells or agents. `x8ai` brings them
  back.
- Logging out or restarting the Mac ends the background `x8ai`, as it ends
  tmux. It is started when `x8ai` is run, not at login.
- After `brew upgrade x8ai`, the running background `x8ai` stays on the old
  version until it is quit or stopped. `x8ai` says so.
- Any process of the user can attach and type into its shells. The user's
  processes can already do anything the user can; nothing outside the user's
  account can.
- One process holds every space. If it crashes, everything in it ends, as when
  `x8ai` crashed before. The terminal says so and where its log is.

## Alternatives considered

- **tmux underneath:** another dependency. Its prefix key, mouse handling and
  status line would fight the Welcome, the panels and Ctrl-g. Spaces,
  approvals and agent panes would still be `x8ai`'s to keep.
- **A launchd agent, started at login:** a plist in `~/Library/LaunchAgents`
  (or `brew services`), and a process running when it is not used. Starting
  it on demand is enough, and nothing is installed outside Homebrew's
  folders.
- **Sending raw keystrokes instead of crossterm's events:** the background
  `x8ai` would need a parser of terminal input of its own. The client already
  has crossterm's, on the terminal it reads.
- **Drawing in every terminal attached:** sizes differ, and keys from two
  places at once need rules. One terminal at a time is what users of a
  workspace expect.
- **Saving programs to disk to restart them:** a running process cannot be
  saved. Keeping it alive is what users want back.
