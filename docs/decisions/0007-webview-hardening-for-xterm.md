# 0007 — Webview hardening adjustments for xterm.js

**Status:** Accepted (Phase 1, 2026-09-28). Amends ADR 0005.

## Context

ADR 0005 enabled `freezePrototype` and a CSP with `style-src 'self'`. It said that
a library breaking under these rules is a reason to evaluate the library, not to
drop the protection. Phase 1 adopted xterm.js (ADR 0006), and both rules broke it:

1. **`freezePrototype`.** xterm.js and its WebGL add-on fail at import with
   `Cannot assign to read only property 'toString'`. They ship a compiled
   TypeScript namespace that exports a function named `toString`
   (`ns.toString = fn` on a plain object). Once `Object.prototype` is frozen,
   JavaScript's "override mistake" rule makes that assignment throw. The whole
   frontend failed to start.
2. **`style-src 'self'`.** xterm.js 6 creates `<style>` elements at runtime for its
   viewport and scrollbar colours, and for the DOM renderer's layout and theme. The
   CSP blocks them. This is visible as unthemed viewport edges, and it would break
   the DOM renderer that serves as the WebGL fallback. Tauri injects nonces and
   hashes into `style-src`, and per the CSP specification that makes
   `'unsafe-inline'` ignored, so adding `'unsafe-inline'` alone is not enough.

Evaluation of the library: xterm.js is the de facto web terminal emulator (VS Code
uses it), and neither behaviour is a defect in it.

## Decision

- Turn **`freezePrototype` off.**
- Allow **inline styles**: `style-src 'self' 'unsafe-inline'`, and
  `dangerousDisableAssetCspModification: ["style-src"]` so Tauri does not add
  nonces there. **`script-src` is unchanged**: Tauri still injects nonces and
  hashes, and inline or remote scripts remain blocked.

## Consequences

- **Prototype pollution is no longer blunted by a frozen prototype.** This matters
  when untrusted data is deep-merged into objects. The frontend currently does no
  such merging. Before any feature processes untrusted structured data in the
  webview (for example, rendering MCP tool results in Phase 7), reconsider a
  targeted defence such as SES-style "override taming", which freezes prototypes
  while still allowing assignments that shadow them.
- **Inline styles are allowed.** Exploiting that requires injecting markup, which
  the app never renders from untrusted sources (`docs/security.md` §3.1, §3.9).
  CSS injection can deface the UI or leak data through selectors, but it cannot
  execute script.
- The main XSS defences are unchanged: `script-src` without `'unsafe-inline'` or
  `'unsafe-eval'`, no untrusted HTML rendering, explicit per-command grants, and no
  shell or fs plugins.

## Alternatives considered

- **Keeping both rules and accepting the breakage:** impossible for
  `freezePrototype`, since the app does not start.
- **Patching xterm.js:** a maintenance burden on a core dependency, for a
  defence-in-depth measure.
- **Custom override taming now:** real protection, but bespoke security code in the
  webview with its own breakage risk. Deferred until untrusted data processing
  makes it worth it.
- **Hashes for xterm's styles:** its `<style>` contents are generated at runtime
  (dimensions, theme colours), so they cannot be hashed ahead of time.
