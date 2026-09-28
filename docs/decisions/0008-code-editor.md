# 0008 — Code editor: CodeMirror 6

**Status:** Accepted (Phase 2, 2026-09-29)

## Context

Phase 2 needs a real code editor: opening, editing and saving files; tabs; syntax
highlighting; search; keyboard navigation; correct Unicode; and large files
without freezing the UI. It must not bring language intelligence (LSP,
completion, semantic analysis) before Phase 9. The editor sits beside the terminal
in a terminal-first app, so it should be lightweight.

The two mature candidates were Monaco (VS Code's editor) and CodeMirror 6.

## Decision

**CodeMirror 6**, from `@codemirror/*`, confined to `src/editor/` (enforced by
`src/architecture.test.ts`).

- **One `EditorView`, one `EditorState` per tab.** Switching tabs swaps the state,
  which carries the text, undo history, selection and folds.
- **The editor store keeps the states, and React sees only tab metadata.** Typing
  never re-renders React.
- **The extension set is explicit** (`src/editor/setup.ts`): line numbers,
  history, folding, bracket matching, multiple selections, search (⌘F, ⌘G, ⌥⌘F)
  and syntax highlighting. It deliberately has no autocompletion, linting or
  language services.
- **Grammars come from `@codemirror/language-data` and load lazily.** Each is a
  separate chunk, loaded the first time a file of that type is shown. Files over
  2 MB open as plain text to stay responsive.
- **Line endings are kept.** A file whose first line break is CRLF is edited and
  saved with CRLF.
- **The theme uses CSS variables,** so it follows the system light and dark
  appearance.

## Consequences

- The core adds about 400 kB to the bundle, and grammars load on demand. There are
  no web workers, so no worker CSP or bundling setup is needed.
- CodeMirror renders only the visible part of a document, which keeps typing
  responsive in multi-megabyte files. Parsing is incremental and time-sliced.
- LSP (Phase 9) and diff views (`@codemirror/merge`) have established CodeMirror
  integrations.
- VS Code keybindings and features beyond this set would have to be added
  explicitly. That is acceptable for an editor that is secondary to the terminal.

## Alternatives considered

- **Monaco:** VS Code parity, excellent large-file handling, and a built-in
  command palette. But it is about 97 MB unpacked and needs web workers (which
  complicates the CSP and bundling). Its TypeScript and JSON language services are
  semantic intelligence that would have to be switched off in Phase 2. It is also
  much heavier than a terminal-first app's secondary editor needs.
- **A plain `<textarea>` or `contenteditable`:** no syntax highlighting, and poor
  performance on large files.
