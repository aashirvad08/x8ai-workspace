# Architecture decision records

One file per significant decision: `NNNN-short-title.md`. A record is never
rewritten after it is accepted. A new record supersedes it.

Each record has these sections: **Status** (Proposed, Accepted, or Superseded by
NNNN), **Context**, **Decision**, **Consequences**, and **Alternatives considered**.

| # | Decision | Status |
| --- | --- | --- |
| [0001](0001-tauri-rust-react.md) | Tauri 2, Rust, React and TypeScript | Accepted |
| [0002](0002-repository-layout.md) | Cargo workspace with Tauri-free crates; `src-tauri` is a thin host | Accepted |
| [0003](0003-rust-owned-ipc-contracts.md) | IPC contracts are defined in Rust and generated for TypeScript | Accepted |
| [0004](0004-integrations-as-external-processes.md) | Integrations are external processes described by declarative definitions | Accepted |
| [0005](0005-webview-trust-boundary.md) | The webview is untrusted; native commands are explicitly granted | Accepted, amended by 0007 |
| [0006](0006-terminal-stack.md) | Terminal stack: portable-pty, xterm.js and Tauri Channels | Accepted, amended |
| [0007](0007-webview-hardening-for-xterm.md) | Webview hardening adjustments for xterm.js | Accepted |
| [0008](0008-code-editor.md) | Code editor: CodeMirror 6 | Accepted |
| [0009](0009-workspace-filesystem-boundary.md) | Workspace filesystem boundary | Accepted |
| [0010](0010-workspace-trust-and-recent-workspaces.md) | Workspace trust and remembered workspaces | Accepted |
| [0011](0011-intercepting-system-quit-on-macos.md) | Intercepting Quit from the Dock, logout and shutdown on macOS | Accepted |
| [0012](0012-agent-runtime.md) | Agent runtime: PTY sessions, the login environment, and per-workspace approval | Accepted, amended by 0015 |
| [0013](0013-agent-worktree-isolation.md) | Agent isolation with Git worktrees, through the user's git | Accepted |
| [0014](0014-secret-storage.md) | Provider credentials in the macOS Keychain, native only | Accepted |
| [0015](0015-environment-precedence.md) | Environment precedence, and what an approval covers | Accepted |
| [0016](0016-provider-and-model-configuration.md) | Providers as data, agents configured by adapters | Accepted |
| [0017](0017-platform-managed-mcp.md) | MCP servers managed by the app, owned by agent sessions | Accepted |
| [0018](0018-catalog-is-discovery-not-execution.md) | The catalog is discovery and orchestration, not execution | Accepted |
| [0019](0019-add-ons.md) | Add-ons: tools for one space's terminals, installed when the user adds them | Accepted |
| [0020](0020-terminal-version.md) | `x8ai`: the workspace in the terminal, installed with Homebrew | Accepted |
| [0021](0021-panes-files-and-the-editor-in-x8ai.md) | Panes, tabs, the file list and the editor in `x8ai` | Accepted |
| [0022](0022-agents-in-x8ai.md) | Agents in `x8ai` | Accepted |
