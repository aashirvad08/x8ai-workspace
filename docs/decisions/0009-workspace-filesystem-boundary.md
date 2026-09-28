# 0009 — Workspace filesystem boundary

**Status:** Accepted (Phase 2, 2026-09-29)

## Context

The editor and file explorer need to read, write, create, rename and delete
files. The webview is untrusted (ADR 0005): anything it can reach, script
running in it (for example through an XSS bug) can reach too. General filesystem
access for the webview, such as the Tauri `fs` plugin with broad scopes, would
put the user's whole home directory within reach.

## Decision

1. **The user chooses the workspace in a native folder picker.** The webview asks
   for the picker (`workspace_open`) but can never name a path to open. Only the
   Rust API of `tauri-plugin-dialog` is used. Its own webview commands are not
   granted. Its injected script only redirects `window.alert` and `confirm` to
   those ungranted commands, which the app never calls.
2. **All file operations live in `crates/workspace`,** a Tauri-free crate, and take
   workspace paths: relative, `/`-separated, validated up front (no absolute
   paths, `..`, `.`, empty segments or NUL).
3. **Every operation runs through a `cap-std` directory handle on the root.**
   cap-std resolves each path beneath the handle and refuses anything that would
   leave it, including through symlinks and on intermediate components. This is
   enforced by construction, not by string checks.
4. **Saves are safe.** An existing file is replaced atomically: a temporary
   sibling is written, flushed to disk and renamed over it, and permissions are
   kept. A save carries the version (modification time and size) it was based
   on. If the file changed or disappeared since then, nothing is written and the
   error is `conflict`. Overwriting takes an explicit second action.
5. **Delete moves entries to the Trash** through `NSFileManager`, so it can be
   undone. The `trash` crate's default Finder/`osascript` method is not used: it
   needs Automation permission and spawns a helper process.
6. **Changes on disk are reported by `notify`** (FSEvents on macOS), batched, and
   delivered on a Channel. Nothing polls. Directories are listed one level at a
   time and only when expanded. Quick open walks the tree on demand with `ignore`
   (respecting `.gitignore`, capped at 50,000 files) and stores nothing.
7. **A page reload closes the workspace,** along with terminals and the quit guard,
   so native state never outlives the page that owned it.

## Consequences

- A compromised webview can read and modify files inside the chosen workspace,
  and nothing else through these commands. That is the intended capability and
  exactly what the user granted by choosing the folder.
- Terminals are not constrained by the workspace boundary. They are the user's
  shell, with the user's privileges (`docs/security.md` §2).
- The atomic replace creates a new inode. Hard links to a saved file are broken,
  and extended attributes are not carried over. Symlinks are written through
  instead of replaced.
- Residual races: a check-then-act window remains between the version check and
  the rename, between the existence check and a rename, and between `cap-std`'s
  check and the Trash call, which uses an absolute path. Each window is short, and
  the operations are the user's own.

## Alternatives considered

- **Tauri `fs` plugin with scopes:** less code, but it gives the webview a general
  API. Its scopes are glob-based string matching, which is harder to reason
  about than a capability handle.
- **Canonicalize-and-prefix-check in our own code:** simple, but easy to get
  wrong with symlinks and races. cap-std is purpose-built and widely used
  (Wasmtime).
- **Permanent delete with a confirmation:** one dependency fewer. Rejected
  because a mis-click in a file tree should be recoverable.
- **In-place writes:** they keep the inode, but a crash or a full disk mid-write
  destroys the file.
