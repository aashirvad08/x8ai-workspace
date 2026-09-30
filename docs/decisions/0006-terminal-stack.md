# 0006 — Terminal stack: portable-pty, xterm.js and Tauri Channels

**Status:** Accepted (Phase 1, 2026-09-28). Proposed in Phase 0. Amended after Phase 7 (see the end): the slave side stays open until the process exits.

## Context

The terminal is the product's primary surface and the substrate agents run on. It
must be a real terminal: login shells, interactive programs, full-screen TUIs,
ANSI and truecolor, resize, Ctrl+C, Ctrl+D and Ctrl+Z, long-running processes, and
many concurrent sessions.

## Decision

- **PTY:** `portable-pty` 0.9 (from the WezTerm project), wrapped by `crates/pty`
  behind our own `Session` and `Sessions` types, so it can be replaced. Its
  `new_default_prog()` resolves the shell (`$SHELL` if executable, then the account
  record, then `/bin/sh`), starts it as a login shell (argv[0] `-zsh`), and makes it
  a session leader with the PTY as its controlling terminal. That setup is what
  makes job control and control keys work without special-casing.
- **Emulator and renderer:** xterm.js 6 (`@xterm/xterm`) in `src/terminal/`, with
  three add-ons: `addon-fit` (size to the container), `addon-webgl` (GPU rendering,
  falling back to the DOM renderer on context loss), and `addon-unicode11` (emoji
  and CJK widths that match modern shells). The clipboard and web-links add-ons are
  deliberately not used (see `docs/security.md` §3.1).
- **Output transport:** one Tauri Channel per session. It carries output as raw
  bytes (`InvokeResponseBody::Raw`, delivered as an `ArrayBuffer`) and lifecycle
  events as JSON. Tauri orders channel messages by index, so an `exited` event can
  never overtake output. Payloads under 1 KiB (keystroke echo) are delivered
  inline; larger ones go through Tauri's fetch path without JSON encoding.
- **Input transport:** `terminal_write` takes the raw request body, with the session
  id in a header. Bytes, including non-UTF-8 mouse reports, reach the PTY unchanged.
  Control keys are plain bytes (`0x03`, `0x04`, `0x1a`), and the kernel's line
  discipline turns them into signals and EOF.
- **Threads per session:** reader, sender, writer and waiter (`crates/pty/src/lib.rs`).
  No IPC command blocks: input is queued to the writer thread.
- **Flow control:** the frontend acknowledges rendered output every 64 KiB. The
  reader pauses while 512 KiB is unacknowledged, which makes the program block on
  write inside the kernel. Output batches are coalesced (at least 4 ms apart), so a
  flood becomes a few large messages per frame.
- **Lifecycle:** close sends SIGHUP to the shell and to the foreground job, then
  SIGKILL to the shell's process group after 2 s. Page reloads close every session.
  App exit hangs up everything and kills survivors after 500 ms. The exit event is
  sent only once output is drained: end of output, or a reader idle for 100 ms when
  a background job keeps the terminal open.

## Findings from implementation

- **Concurrent `openpty` fails on macOS.** Parallel calls intermittently failed with
  `Unknown error: -6`, so PTYs are opened under a process-wide lock. The
  regression test spawns 48 sessions from 48 threads and detected the missing lock
  in 6 of 8 runs.
- **A theory disproved.** Output of a short-lived process seemed lost once. I
  suspected macOS discards buffered PTY output when the last slave handle closes,
  and tested that directly (reader delayed 50 ms, slave closed early). The output
  survived, so the workaround was reverted. The loss was a symptom of the `openpty`
  race.
- **xterm.js needs two webview hardening changes** (ADR 0007).
- **GUI environment.** A Finder-launched app has no `LANG`, so `crates/pty/src/locale.rs`
  sets it from the macOS language and region when nothing names a locale, as
  Terminal.app does. `PATH` needs no help: the login shell rebuilds it (verified
  under a scrubbed environment).

## Consequences

- The native side never parses ANSI. Terminal fidelity is xterm.js's.
- The same session substrate will host coding agents in Phase 4 (`Program::Exec`).
- The frontend bundle is about 700 kB, most of it xterm.js. It is loaded from disk,
  so bundle size is not a latency concern.

## Alternatives considered

- **PTY:** `nix`/`rustix` `openpty` directly (fewer layers, but Unix-only, and more
  of our own unsafe code), or `pty-process`. portable-pty already did everything
  needed and is widely used.
- **Emulator:** a WebAssembly build of a native terminal core (for example from the
  Ghostty project). It is worth revisiting if xterm.js fidelity or performance
  falls short.
- **Native GPU rendering** with the webview only for chrome. It gives the best
  performance, at a large cost in complexity and platform-specific code.

## Amendment (after Phase 7)

The "theory disproved" above was only half disproved. Closing the slave side
early from the app does not lose output. But when the program's own exit is the
terminal's last close, macOS discards output not yet read: the reader then gets
end of output with nothing read. It shows only when the reader has not run
before the program exits: under load, as on a CI runner. The app now keeps the
slave side open until the process has exited and closes it itself, so the
output is kept until it is read. The idle fallback for terminals held open by a
background job also checks that nothing is waiting to be read, closing the
timing gap noted in `docs/architecture.md`. The regression test
`short_lived_output_survives_a_starved_reader` starves the readers with busy
threads: it fails on every run without the change, and passes with it.
