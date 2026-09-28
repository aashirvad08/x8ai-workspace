# 0001 — Tauri 2, Rust, React and TypeScript

**Status:** Accepted (Phase 0, 2026-09-28)

## Context

We need a macOS-first desktop app that manages PTYs and processes, stores secrets
and integrates with the OS. It also needs a rich, fast-to-iterate UI and should
keep Linux possible. Native process work must be robust and memory-safe.

## Decision

- **Tauri 2** (stable 2.x line; 2.12 at the time of writing) as the application
  framework. Tauri 3 is in alpha and is not used.
- **Rust** for everything native: PTYs, processes, filesystem, secrets, OS
  integration and native state.
- **React 19 + TypeScript** (strict) for UI, built with **Vite**.
- **pnpm** as the package manager, pinned through `packageManager`.
- **Vitest** for frontend tests, because it reuses the Vite config.
- The Rust toolchain is pinned in `rust-toolchain.toml` so local builds and CI use
  the same compiler and clippy.
- Minimum macOS 13. WKWebView's engine is tied to the OS, and older releases lack
  the modern JavaScript and CSS the frontend targets.

No other runtime dependencies are added in Phase 0. We deliberately left out a CSS
framework, a state library, a router and ESLint. Frontend boundary rules are
enforced by `src/architecture.test.ts` instead of a lint plugin.

## Consequences

- The app uses the system WebKit, not a bundled Chromium. The bundle is small
  (about 10 MB), but web platform features follow the macOS version. We must
  test on the minimum OS.
- Rust gives us memory-safe process and PTY code, and access to the mature
  ecosystem around terminals (`portable-pty` from WezTerm).
- Tauri's capability system gives us a real, enforced IPC allowlist (ADR 0005).
- Linux is possible later through webkit2gtk. Its rendering differences need a CI
  job (Phase 12).

## Alternatives considered

- **Electron:** mature, with Chromium everywhere. But it is heavier, its native
  work is in Node (a weaker fit for PTY and process supervision than Rust), and
  its larger default attack surface needs careful hardening.
- **Native Swift/AppKit:** best macOS integration, but no Linux path, and slower UI
  iteration for editor, terminal and catalog surfaces that web tech already
  serves well (xterm.js, CodeMirror).
- **Tauri 3 alpha:** not stable. Revisit when it is released.
