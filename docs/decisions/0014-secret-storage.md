# 0014. Provider credentials in the macOS Keychain, native only

**Status:** Accepted (Phase 6)

## Context

Phase 6 lets the user save provider API keys once and use them from agents. A key
is valuable, long-lived and easy to leak: into a config file, a log line, an error
message, the webview's memory or storage, a worktree an agent commits, or the
project itself. Four questions:

1. **Where** are keys stored?
2. **Who** can read them: the webview, the native side, the agent?
3. **How** are they kept out of logs, errors and files by accident, not just by
   care?
4. **What** dependency does it take?

## Decision

**The login Keychain, one generic password per provider** (service
`com.x8ai.workspace.providers`, account = the provider's secret name, a readable
label). The Keychain encrypts at rest, is unlocked with the user's login, is
visible and removable in Keychain Access, and belongs to the app that created the
item. Items are not synchronizable, so they stay on this Mac.

**The webview can write and delete, never read.** It sends a key once, when the
user saves it; the command returns the provider's status. No command returns a
key. The webview learns only whether one is saved (`notNeeded`, `missing`,
`inKeychain`), which is checked without reading it. The native side reads a key
only when the agent of a session using that provider starts, and places it only
in that agent's environment (ADR 0015). *Amended:* the key is then kept in the
app's memory until the app quits or it is replaced or removed, so macOS asks for
the Keychain password at most once per app run instead of at every launch.

**A type that cannot leak by accident.** `SecretValue` (`crates/secrets`) holds a
key in memory: no `Display`, no `Serialize`, a redacted `Debug`. Construction checks
the input (not empty, at most 4096 bytes, one line, no control characters) with
errors that never contain it. Every type that can carry a key in an environment
(`LaunchPlan`, the adapter's `Configuration`, the PTY's `Environment`) prints
variable names only.

**`security-framework`**, the standard Rust binding to Apple's Security framework,
with default features off (no TLS, no extra APIs), macOS only. Other platforms get
an explicit "no secret store yet" error rather than a weaker store.

## Consequences

- No file the app writes holds a key: not `providers.json`, the approval and trust
  stores, worktree metadata, the project or a worktree. Tests scan every file a
  session touched for the key.
- A key survives restarts and app updates. Removing it in the app deletes the
  Keychain item; an agent already running keeps the copy in its environment.
- Development builds are signed differently on each build; macOS may ask whether a
  new build may read an item the previous one created. A released, consistently
  signed app does not ask.
- The Keychain can block (for example while macOS asks to unlock it), so every
  Keychain call runs off the IPC thread.
- Linux (Phase 12) needs a Secret Service backend behind the same trait.

## Alternatives considered

- **A file encrypted by the app.** The app would have to keep the encryption key
  somewhere, which is the same problem again, and it would be one more format to
  get right. Rejected.
- **`localStorage` or IndexedDB in the webview.** Readable by any script in the
  page, unencrypted on disk. Rejected.
- **Reading keys from the user's shell** (their `ANTHROPIC_API_KEY`). That is what
  "the agent's own configuration" already does; storing a key in the app is for
  users who do not want it in every shell. Both stay available; they are not
  mixed (ADR 0015).
- **The `keyring` crate.** Cross-platform, but a larger surface and an extra layer
  over the same Security framework calls. Revisit for Linux.
- **Passing keys to agents through files** (a generated settings file). Files
  outlive processes and can end up in a commit. Rejected: environment only.
