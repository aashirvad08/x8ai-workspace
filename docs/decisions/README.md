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
| [0006](0006-terminal-stack.md) | Terminal stack: portable-pty, xterm.js and Tauri Channels | Accepted |
| [0007](0007-webview-hardening-for-xterm.md) | Webview hardening adjustments for xterm.js | Accepted |
| [0008](0008-code-editor.md) | Code editor: CodeMirror 6 | Accepted |
| [0009](0009-workspace-filesystem-boundary.md) | Workspace filesystem boundary | Accepted |
| [0010](0010-workspace-trust-and-recent-workspaces.md) | Workspace trust and remembered workspaces | Accepted |
| [0011](0011-intercepting-system-quit-on-macos.md) | Intercepting Quit from the Dock, logout and shutdown on macOS | Accepted |
| [0012](0012-agent-runtime.md) | Agent runtime: PTY sessions, the login environment, and per-workspace approval | Accepted |
| [0013](0013-agent-worktree-isolation.md) | Agent isolation with Git worktrees, through the user's git | Accepted |
