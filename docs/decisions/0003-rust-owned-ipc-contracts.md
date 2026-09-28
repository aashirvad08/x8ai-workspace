# 0003 — IPC contracts are defined in Rust and generated for TypeScript

**Status:** Accepted (Phase 0, 2026-09-28)

## Context

Every IPC payload crosses the trust boundary between the webview and the native
host. If the types are defined twice, once in Rust and once in TypeScript, they
drift. Drift at a trust boundary produces bugs that look like security issues
(fields silently ignored or misread).

## Decision

- Contract types are defined **once, in Rust**, in `x8ai-core`, with serde
  attributes that fix the wire format (camelCase, tagged unions).
- **`ts-rs`** derives TypeScript declarations. `cargo test` writes them to
  `src/contracts/generated/` (the path is set through `TS_RS_EXPORT_DIR` in
  `.cargo/config.toml`). `pnpm contracts` regenerates only the bindings.
- Generated files are **committed**. CI runs the tests and fails if
  `src/contracts/generated/` differs from the committed copy.
- The frontend imports these types only as types. Native return values are
  trusted and cast. Values sent to native are validated in Rust.
- ts-rs's warnings about serde attributes it does not interpret are silenced
  (`no-serde-warnings`). The attributes currently in use (`try_from`/`into` on
  string newtypes, `deny_unknown_fields`) do not change the TypeScript shape.
  **Review rule:** any new serde attribute that affects the wire shape (`flatten`,
  `with`, `skip`, custom (de)serializers) must come with a check of the generated
  file in the same change.

## Consequences

- One source of truth. Changing a Rust contract produces a TypeScript compile error
  wherever the frontend relies on the old shape. For example, `NativeError` uses
  `Record<ErrorCode, true>`, so a new error code breaks the build until it is
  handled.
- Command names are still repeated in three places (`generate_handler!`,
  `build.rs`, and the TypeScript client). The checklist in
  `docs/architecture.md` §4 covers this. If it becomes a source of bugs, adopt
  typed command generation (`tauri-specta`) in a new ADR.
- `ts-rs` is a build-time-only concern in practice. It adds derive code but no
  runtime behaviour.

## Alternatives considered

- **Hand-written TypeScript types:** no dependency, but drift is certain.
- **`tauri-specta`:** also generates typed command wrappers. It is attractive, but
  its Tauri 2 line was still a release candidate (`2.0.0-rc.25`) when evaluated,
  and it couples contract generation to the Tauri crate. Revisit when it is
  stable.
- **JSON Schema as the source of truth:** language-neutral, but it adds a code
  generation step on both sides for no present benefit.
