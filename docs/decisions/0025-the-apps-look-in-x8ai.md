# 0025. The app's look in `x8ai`

**Status:** Accepted

## Context

`x8ai` (ADR 0020 to 0024) had the app's screens and keys, but not its look.
It drew in the terminal's own colors, with the app's raspberry and indigo
only for accents and dimmed text for everything else. The Welcome greeting
was one row of spaced capitals. Bars, panels and boxes had no backgrounds,
and selections were reversed text. Side by side with the app (src/app/app.css)
it looked like a different product. OpenCode shows that a terminal program
can look designed: a terminal is a grid of characters, but every cell has its
own background, and 24-bit color and box-drawing characters go a long way.

The questions:

1. Whose colors: the user's terminal theme, or the app's?
2. What becomes of the app's large "WELCOME, SIR" in a grid of one font size?
3. How do bars, panels, buttons and inputs map onto cells?
4. What about terminals without 24-bit color, and light mode?

## Decision

**x8ai paints the app's palette on every cell.** The background, raised
surfaces, borders, text, the indigo accent and fill, the raspberry highlight,
and the green and red are the app's tokens (`--bg`, `--bg-raised`, …). They
live in `theme.rs`, dark or light as macOS is (`AppleInterfaceStyle`, read
when a terminal attaches). Panes get the app's terminal palette too
(src/terminal/theme.ts): its 16 colors, foreground, background and selection.
So a shell in x8ai shows the colors it shows in the app. A program that sets
exact colors (24-bit, or 256-color indexes above 15) keeps them.

When a terminal attaches, the background `x8ai` also sets that terminal's own
background and cursor color to the app's (OSC 11 and 12), so the margin around
the grid matches. The terminal puts its own back when `x8ai` lets go (OSC 111
and 112).

**The greeting is drawn with lines.** "WELCOME," uses light box-drawing lines
and "SIR" heavy ones, in raspberry, followed by a two-column block cursor. The
app sets them in light and bold weights of the same capitals. It takes three
rows and 63 columns. Where there is no room, it falls back to one row of
spaced capitals. The name under it is spaced capitals in raspberry, as in the
app.

**The app's parts, in cells.**
- The command line is a raised box with an indigo bar (`▎`) on its left.
- Suggestions sit under it on a grey bar.
- Keys show as chips: bold, on the active grey.
- RECENT SPACES is a heading in faint capitals, with names in indigo.
- A space has a raised tab bar on top, with the open tab on the app's
  background. At the bottom is the app's status bar: `⌂`, a green dot, the
  keys that work now, then the space and **Trusted** or **Untrusted**. Trust
  moved there from the tab bar, as in the app.
- The sidebar is raised. A selected row is on the active grey, in indigo.
- Questions, forms and pickers are the app's dialogs: raised, a rounded
  border, buttons on the right. The one that agrees is white on the indigo
  fill. Inputs are rows on the app's background.

**Exact where the terminal allows.** With 24-bit color (`COLORTERM`, set by
iTerm2, Ghostty, kitty, WezTerm, and by Terminal on macOS 26), every color is
the app's. Elsewhere each is the nearest of the 256 colors, which keeps the
greys apart.

## Consequences

- x8ai looks like the app in every terminal, whatever the terminal's theme.
  The user's terminal palette no longer shows inside x8ai, including in its
  shells, as it doesn't in the app.
- Text checks in tests changed. A space's trust is now read from the status
  bar ("proj  Untrusted"). The greeting is lines, not letters. The Welcome
  shows the same status bar, so tests tell a space by its tab bar.
- Fonts without the heavy box-drawing characters would show SIR uneven; SF
  Mono and Menlo, Terminal's and iTerm2's defaults, have them.

## Alternatives considered

- **Keeping the terminal's background and colors:** fits a terminal theme,
  but never looks like the app, which was the point. The user chose the app's
  look.
- **A greeting in block letters (`█▀▄`):** reads heavier than the app's light
  capitals, and needs as many rows.
- **Following the terminal's light or dark background instead of macOS:**
  terminals don't report it reliably. The app follows macOS, so x8ai does
  too.
