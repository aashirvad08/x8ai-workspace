# 0002 — Cargo workspace with Tauri-free crates; `src-tauri` is a thin host

**Status:** Accepted (Phase 0, 2026-09-28)

## Context

The app will grow several native subsystems: PTY sessions, workspace and
filesystem, agents, secrets, providers, MCP, git and catalog. If they all live
inside the Tauri crate, boundaries hold only by convention, every test compiles
the webview stack, and nothing can be reused without Tauri.

## Decision

- The repository root is a **Cargo workspace**.
- `crates/core` (`x8ai-core`) holds the contracts: IPC types and integration
  definitions. It has **no Tauri dependency, no I/O and no process management**,
  and it forbids `unsafe`.
- Each native subsystem becomes its own crate under `crates/` **in the phase that
  implements it** (for example `crates/pty` in Phase 1). It depends on `x8ai-core`,
  never on Tauri.
- `src-tauri` (`x8ai-desktop`) is the only crate that depends on Tauri. It wires
  crates to IPC commands, holds native state and owns the window.
- The frontend lives in `src/`, with the standard Tauri layout at the root.
  Frontend feature folders (`src/terminal/`, `src/editor/`, …) are created with
  their phase, not in advance.

## Consequences

- The compiler enforces that domain logic cannot reach into Tauri or the webview.
- `cargo test -p x8ai-core` is fast and runs on any OS. CI runs it on Linux to keep
  the core portable.
- A future CLI or headless test harness can reuse the crates.
- There are more `Cargo.toml` files to maintain. Shared versions live in
  `[workspace.dependencies]`, which keeps them consistent.

## Alternatives considered

- **Everything in `src-tauri/src/` as modules:** simplest at first, but boundaries
  are not enforced and tests need the full Tauri build.
- **A monorepo with separate JavaScript packages** (`packages/ui`,
  `packages/terminal`, …): premature for one app. It adds build orchestration
  without a present benefit. Revisit if another frontend (for example a web
  companion) appears.
- **Creating all subsystem crates and folders now:** rejected. Empty modules imply
  structure that has not been designed yet.
