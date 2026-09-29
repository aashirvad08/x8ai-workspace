# x8ai Workspace

A terminal-first AI development workspace for macOS.

The terminal is the primary surface. Coding agents such as Claude Code, OpenCode,
Codex and Aider run inside it. Model providers (Anthropic, OpenAI, Google,
OpenRouter, Ollama and local models), MCP servers, Git worktrees and an
installable catalog are organized around it. The app hosts and orchestrates these
existing tools. It does not implement its own LLM or its own coding agent, and it
does not privilege any vendor.

> **Status: Phase 6 (model providers).** Open a folder to get a file explorer,
> workspace search and a code editor above real PTY terminals. Coding agents such
> as Claude Code run in the folder's terminals once you trust the folder and allow
> the agent there; in a Git repository each agent works in a worktree of its own,
> and you review what it changed. Save provider keys once, in the macOS Keychain,
> and start an agent with a provider and model of your choice, or with its own
> configuration. The app does not implement an agent or a model. See
> [docs/roadmap.md](docs/roadmap.md), [docs/agent-runtime.md](docs/agent-runtime.md),
> [docs/multi-agent.md](docs/multi-agent.md) and [docs/models.md](docs/models.md).

## Documentation

- [Architecture](docs/architecture.md): subsystems, boundaries, process model and
  future designs
- [Roadmap](docs/roadmap.md): Phases 0–12 with acceptance criteria
- [Security](docs/security.md): threat model, risks, and what is enforced today
- [Agent runtime](docs/agent-runtime.md): how agents are found, approved, started and stopped
- [Multi-agent workspaces](docs/multi-agent.md): agent sessions, worktrees and review
- [Models and providers](docs/models.md): providers, models, keys, precedence and agent adapters
- [Decisions](docs/decisions/): architecture decision records

## Prerequisites

- macOS 13 or later, with Xcode Command Line Tools (`xcode-select --install`)
- Node.js 22.12 or later, and pnpm 11 (`corepack enable`, or see
  `packageManager` in `package.json`)
- Rust through [rustup](https://rustup.rs). The exact toolchain is pinned in
  `rust-toolchain.toml` and installed automatically.

## Commands

| Command | What it does |
| --- | --- |
| `pnpm install` | Install frontend dependencies |
| `pnpm tauri dev` | Run the app with hot reload |
| `pnpm tauri build` | Build the unsigned `target/release/bundle/macos/x8ai Workspace.app` |
| `pnpm tauri build --bundles dmg` | Also build a DMG. This drives Finder through AppleScript, so your terminal needs Automation → Finder permission. Signed and notarized DMGs come in Phase 12. |
| `pnpm check` | Everything CI checks: typecheck, frontend tests, rustfmt, clippy, cargo tests |
| `pnpm test` | Frontend tests (Vitest), including module-boundary tests |
| `cargo test --workspace` | Rust tests. Also regenerates the TypeScript contracts. |
| `pnpm contracts` | Regenerate `src/contracts/generated/` from `crates/core` |

## Keyboard shortcuts

⌘O Open Folder · ⌃R Open Recent · ⌘P Go to File · ⇧⌘P All Commands · ⌘N New File ·
⌘S Save · ⌥⌘S Save All · ⌘W Close Editor · ⌘B Toggle Sidebar · ⇧⌘E Files ·
⇧⌘F Search in Folder · ⇧⌘A Agents · ⇧⌘M Models · ⌘F Find in File · ⌃` Toggle Terminal · ⌃⇧` New Terminal.
In a terminal: ⌘D Split Right · ⇧⌘D Split Down · ⌘] / ⌘[ Next / Previous Pane ·
⌘W Close Pane. In the explorer: arrows, Enter, F2 to rename, ⌘⌫ to move to the
Trash.

## Repository layout

```
crates/core/        x8ai-core — IPC contracts and integration definitions (no Tauri, no I/O)
crates/pty/         x8ai-pty — PTY sessions: the user's shell, streamed output, lifecycle (no Tauri)
crates/workspace/   x8ai-workspace — the chosen folder, scoped file operations, search, recent, trust, approvals (no Tauri)
crates/agents/      x8ai-agents — agent definitions, discovery, sessions, worktree isolation, runtime, model adapters (no Tauri)
crates/git/         x8ai-git — the user's git: repository facts, worktrees, changes (no Tauri)
crates/secrets/     x8ai-secrets — provider keys in the macOS Keychain; a value that cannot be printed (no Tauri)
crates/providers/   x8ai-providers — provider definitions, models, provider settings, Ollama detection (no Tauri)
src-tauri/          x8ai-desktop — the Tauri host: window, commands, capability grants
src/app/            UI shell: layout, sidebar, splitters, overlays, shortcuts (React)
src/workbench/      user actions and coordination; notifications, dialogs, commands
src/workspace/      file explorer and workspace search
src/editor/         editor tabs and CodeMirror
src/terminal/       terminal sessions, tabs, split panes and xterm.js view
src/agents/         agents view and store: agents, sessions, review
src/models/         models view and store: providers, keys, model ids
src/native/         typed client for native commands; the only code that imports Tauri
src/contracts/      TypeScript types generated from crates/core (do not edit)
docs/               architecture, roadmap, security, decisions
```

Subsystem modules (MCP and catalog) are added in the phase
that implements them. The planned homes are in
[docs/architecture.md §3](docs/architecture.md#3-repository-map).

## Development rules

1. Keep subsystems separate. No business logic in UI components.
2. Prefer simple designs to premature abstraction. Build features in their phase.
3. Justify every new dependency in the change that adds it.
4. Keep every commit small and buildable. Never hide errors.
5. No mock implementations posing as real functionality. Mark future work with
   `TODO(phase-N)`.
6. Keep platform-specific code isolated. The platform is macOS first, with Linux
   kept possible.
7. Never hard-code credentials. Never couple the app to a single agent, provider or
   MCP server.
8. Changes to `src-tauri/capabilities/` are security changes. Review them as such.
