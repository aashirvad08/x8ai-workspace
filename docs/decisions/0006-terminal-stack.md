# 0006 — Terminal stack: portable-pty, xterm.js and Tauri Channels

**Status:** Proposed. To be accepted or replaced in Phase 1 after a spike.

## Context

The terminal is the product's primary surface and the substrate agents run on. It
must be a real terminal: login shells, interactive programs, full-screen TUIs,
ANSI and truecolor, resize, Ctrl+C and Ctrl+D, long-running processes, and many
concurrent sessions. Phase 0 does not implement it, but the architecture must not
need restructuring when it arrives.

## Decision (proposed)

- **PTY:** `portable-pty` (the WezTerm project's PTY crate, 0.9.x) in a new
  `crates/pty`. It covers the POSIX PTY APIs on macOS and Linux (and ConPTY
  should Windows ever matter), process spawning into the PTY, and resize. It
  lives behind our own session interface (architecture §6), so it can be swapped
  out.
- **Emulator and renderer:** **xterm.js** (`@xterm/xterm` 6.x) in the webview, with
  the fit add-on and the WebGL renderer where available. It is the de facto
  standard (VS Code uses it) and handles escape sequences, the alternate screen,
  mouse reporting, and Unicode and wide characters.
- **Output transport:** a Tauri **Channel** per session (`tauri::ipc::Channel`),
  which is ordered and designed for streaming. A native reader thread batches
  output by size or time. Bounded buffers apply backpressure.
- **Input transport:** a `terminal_write(session, bytes)` command. Control keys are
  plain bytes (`0x03`, `0x04`). The kernel's line discipline does the rest.
- **Sessions** are owned by a native registry. The webview holds only ids.

## Consequences

- The native side stays free of ANSI parsing, so there is less code and less
  attack surface. Terminal fidelity is what xterm.js provides.
- The IPC cost of per-keystroke `invoke` and batched output must be measured. The
  spike must show typing latency and throughput (for example `cat` of a 100 MB
  file, and `yes`) that are acceptable without freezing the UI.
- The frontend `src/terminal/` module will import xterm.js, which is the first
  large frontend dependency. That is justified by the requirements and must be
  noted when it is added.

## Alternatives to evaluate in the spike

- **PTY:** `nix`/`rustix` `openpty` directly (fewer layers, but Unix-only, and
  more of our own code to maintain), or the `pty-process` crate.
- **Emulator:** a WebAssembly build of a native terminal core (for example from the
  Ghostty project) for fidelity and speed, if one is stable enough.
- **Native rendering** (a GPU terminal in Rust with the webview only for chrome):
  best performance, but a large increase in complexity and platform-specific
  code. This is not proposed.
