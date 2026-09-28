# 0005 — The webview is untrusted; native commands are explicitly granted

**Status:** Accepted (Phase 0, 2026-09-28)

## Context

From Phase 1, native commands will be able to write into live shells. Anyone who
can run script in the webview could then run arbitrary commands as the user. The
webview will render content we do not control: file names, tool output, agent
Markdown and MCP descriptions. XSS in this app is therefore equivalent to remote
code execution.

## Decision

- Treat the webview as an **untrusted client** of the native host.
- **Explicit command grants.** Every app command is listed in `src-tauri/build.rs`,
  so Tauri generates permissions with no default grant. A command runs only when a
  capability file in `src-tauri/capabilities/` grants it to the specific window.
- **No broad plugins.** No shell, fs, http or opener plugins. Native capabilities
  are exposed as narrow, purpose-built commands that validate their arguments.
- **Strict CSP.** No remote scripts, no inline scripts, no `eval`, no plugins or
  frames, and `connect-src` restricted to IPC. `unsafe-inline` styles are allowed
  only in the dev CSP, for Vite's hot module reload.
- **`freezePrototype: true`**, to blunt prototype-pollution gadgets.
- `withGlobalTauri` stays off. Only `src/native/` imports the Tauri API, which is
  enforced by `src/architecture.test.ts`.
- Future windows that render untrusted content get their own capability, with the
  smallest possible grant (ideally none).

## Consequences

- Adding a command takes deliberate steps (architecture §4), and each capability
  diff reads as a security diff.
- This was verified in Phase 0. With the grant removed, the call is rejected with
  `Command get_app_info not allowed by ACL` before the command's code runs, and the UI
  shows the error.
- A future UI feature that "just needs" general filesystem or shell access must be
  redesigned as a narrow command instead.
- `freezePrototype` can break libraries that assign to built-in prototype
  properties. Such breakage is treated as a reason to evaluate the library, not to
  disable the protection.

## Alternatives considered

- **Tauri default capabilities with plugins (shell, fs):** faster to build on, but
  it gives the webview broad OS access, and it relies on per-plugin scope
  configuration that is easy to get wrong.
- **Tauri's isolation pattern** (an iframe that validates IPC messages): useful
  when third-party frontend code is present. We ship none today, so it adds
  complexity without a current benefit. Revisit if untrusted frontend code is
  ever loaded.
