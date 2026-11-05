# Working on x8ai Workspace

Notes for Claude Code. The README's "For contributors" section has the commands
and ground rules, and `docs/decisions/` has the reasoning. This file holds what
they don't say. The repo is public, so keep secrets and personal details out.

## How to work here

- On a multi-step plan, finish one step, commit and push it, say plainly that it
  is complete, and wait for approval before starting the next. Never roll into
  the next step on your own.
- Work lands directly on `main`; there have been no pull requests so far. Push
  only when asked.
- Commits look like `feat(tui): …`, `test(tui): …` and `docs: …`, with code and
  docs in separate commits.

## Where things stand

- The five-step plan to ship x8ai as a terminal program through Homebrew is done
  (ADRs 0020 to 0024). v0.6.0 gave it the desktop app's look (ADR 0025).
- The Tauri desktop app still works and shares x8ai's data. It may be removed
  later; that is undecided. The README is written for the terminal version.

## Releasing

1. Bump `version` in `crates/tui/Cargo.toml` (x8ai has its own version, not the
   workspace's), and run `cargo build -p x8ai` to update `Cargo.lock`.
2. Push to `main` and wait for CI to pass before tagging, because a release can't
   be cleanly taken back. CI takes 15 to 20 minutes.
3. Make an annotated tag, `git tag -a v<version> -m "x8ai <version>: <summary>"`,
   and push only that tag. The release workflow fails if it doesn't match the
   crate's version.
4. Check whether the workflow's "Update the tap" step ran. It was skipped through
   v0.7.0 because the `HOMEBREW_TAP_TOKEN` secret isn't set. If skipped, update
   Homebrew by hand:
   - Take the checksum from the release's `.sha256` file.
   - Run `scripts/release/homebrew-formula.sh <version> <sha256> aashirvad08` and
     write the output to `Formula/x8ai.rb` in
     `github.com/aashirvad08/homebrew-tap`.
   - The diff should be three lines: `url`, `version` and `sha256`. Commit it as
     `x8ai <version>` and push.
5. Check it with `brew fetch --formula aashirvad08/tap/x8ai`, which downloads the
   package and verifies its checksum without installing it.

## Testing x8ai

Prefer the Rust PTY harness in `crates/tui/tests/common`, which reads the screen
through a terminal emulator:

- `X8ai` stops the test's background server on drop; `hang_up()` closes the
  terminal but keeps the server.
- Wait for a space with `wait_for_space`, because the Welcome's status bar names
  the space too.
- The debug-only `X8AI_TEST_SECRETS` and `X8AI_TEST_MCP_SOCKETS` keep tests away
  from the Keychain.

### Running the real program by hand

Set `HOME` and `X8AI_DATA_DIR` to a `mktemp -d` folder so the real data stays
untouched, keeping the real `CARGO_HOME` and `RUSTUP_HOME` if going through cargo.
The server's socket path has to fit macOS's 104-byte limit, so avoid long
temporary paths. Afterwards, check that no `x8ai --server` is left running.

The Welcome behaves differently from a space:

- Inside a project folder it reads "Enter opens <folder>, or type a command…".
  "Type a command" appears only with no folder to offer.
- Ctrl-g is a prefix only inside a space. On the Welcome, `q` lands in the command
  line; quit with `/quit`, or Ctrl-c on an empty line.

### Pitfalls with `expect`

These once made a broken smoke test report success, so make every step fail
loudly, with `timeout` and `eof` branches:

- Write `expect { ... }` blocks over several lines. On one line, the whole braced
  text is a single literal pattern: nothing matches, it waits out the timeout, and
  it carries on as if all were fine.
- Set `set stty_init "rows 30 columns 100"` before `spawn`, or the terminal is 0x0
  and x8ai draws nothing.
- Tcl reads `\x07f` as hex 7f. Write Ctrl-g followed by a hex letter as `\007f`.
- Match single words. ratatui writes only the cells that changed, so words
  separated by spaces aren't contiguous in the raw output.
- With `log_user 0`, `log_file` records nothing unless given `-a`.
- An exact, tiny environment (no `USER` or `TMPDIR`) can make zsh misbehave. Pass
  the real variables when checking real agents.

### Reading results

A pipeline's exit status is its last command's, so a job ending in `| tail`
reports 0 even when the checks failed. Read each step's exit status from its log.
