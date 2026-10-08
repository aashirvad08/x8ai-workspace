# 0021. Panes, tabs, the file list and the editor in `x8ai`

**Status:** Accepted

## Context

Step 1 of the terminal version (ADR 0020) showed a space with one shell. A
workspace needs what the app's has: several terminals side by side and in
tabs, the folder's files, and a way to edit them. In a terminal there is no
CodeMirror, and the mouse belongs to the user's terminal unless the program
asks for it. The questions:

1. How are panes and tabs laid out, and moved between?
2. How are files listed, and kept current?
3. Which editor opens a file, and how is it started?
4. What happens when a program in a pane ends?
5. Does `x8ai` take the mouse, and what is lost if it does?

## Decision

**Tabs of pane trees, as in the app.** Each space has tabs; each tab is a
binary tree of panes split to the right or below (`layout.rs`, the same model as
`src/terminal/panes.ts`, on the grid of cells). Panes side by side are
separated by a column of `│`; when a tab has more than one pane, each gets a
title row, which also separates panes stacked one above the other. The keys
follow the Ctrl-g prefix and tmux where it is familiar: `t` a new tab, `n` `p`
`1`–`9` between tabs, `|` and `-` (and tmux's `%` and `"`) to split, the arrows
and `o` between panes, `z` to zoom one, `x` to close one (asking first while a
program runs), `?` for every key. The layout lives only while `x8ai` runs.

**The file list reads the folder through `x8ai-workspace`.** One folder at a
time, as the app's explorer does, so it cannot leave the space; the space's
watcher reloads it on changes. Ctrl-g `f` shows it and gives it the keys: the
arrows (and `hjkl`) move and open or close folders, Enter opens a file, Esc
goes back to the terminal.

**Files open in the user's editor, as git opens it.** A file is opened in a tab
of its own running `sh -c 'exec ${VISUAL:-${EDITOR:-vi}} "$1"' x8ai-editor
<file>`: `$VISUAL` or `$EDITOR` may carry options (`code -w`), the script is
constant, and the file's path is an argument, never part of it. A file already
open in a running editor shows that tab instead of opening another.

**A program that ends well takes its pane with it.** As a terminal tab closes
when its shell exits, a pane closes when its program exits with 0: `exit` in a
shell, `:wq` in the editor. One that fails or is killed stays, with a bar that
says how it ended, so its output can be read; Enter starts a new shell in its
place, or closes the editor's pane. When a space's last pane closes, the
Welcome shows and Esc starts a new shell.

**`x8ai` takes the mouse, and gives back what that costs.** It turns on mouse
reports for presses, releases and drags (1000, 1002, in SGR form, 1006), not
for every movement. A click focuses a pane or a tab, dragging the line between
panes resizes them, and the wheel scrolls the file list or a pane's scrollback.
A program that asked for the mouse (vim, htop) gets its own reports, with
positions inside its pane; the wheel sends arrow keys to one that shows its own
screen without asking (less, man). Because the terminal's own selection no
longer works by plain dragging, `x8ai` selects itself: dragging over a pane's
text highlights it and copies it to the clipboard with `pbcopy` when released
(Shift-drag does this in a program that uses the mouse). Nothing reads the
clipboard.

## Consequences

- A space in `x8ai` now holds what a session needs: shells side by side and in
  tabs, the files, and an editor, without leaving the terminal.
- The user's editor is their own, with their configuration; `x8ai` has no
  editor of its own to maintain. A GUI editor started without waiting (`code`
  without `-w`) opens the file and its pane closes at once.
- Selecting with the terminal's own selection needs the terminal's modifier
  (Option in iTerm2 and Terminal.app) while `x8ai` has the mouse; `x8ai`'s own
  selection copies on release instead.
- Splits are resized with the mouse only; there are no resize keys yet. The
  layout is not kept when `x8ai` quits (a later step, with the background
  process).

## Alternatives considered

- **An editor of `x8ai`'s own:** a large surface to build and keep safe, and
  never as good as the editor the user already has.
- **Opening a file in a split beside the focused pane:** where it lands then
  depends on the layout; a tab of its own is predictable, and splitting stays
  one key away.
- **Leaving the mouse to the terminal:** no clicking panes or tabs, no dragging
  dividers, and the wheel in a pane's alternate screen would move the user's
  terminal instead of the program.
- **A copy mode driven by keys (tmux's):** more to learn; dragging is what
  people try first. Scrolling back with keys stays (Ctrl-g `s`).
