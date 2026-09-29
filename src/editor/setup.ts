import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  bracketMatching,
  foldGutter,
  foldKeymap,
  indentOnInput,
  LanguageDescription,
  syntaxHighlighting,
} from "@codemirror/language";
import { languages } from "@codemirror/language-data";
import { highlightSelectionMatches, search, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, type Extension } from "@codemirror/state";
import {
  crosshairCursor,
  drawSelection,
  dropCursor,
  highlightActiveLine,
  highlightActiveLineGutter,
  highlightSpecialChars,
  keymap,
  lineNumbers,
  rectangularSelection,
} from "@codemirror/view";

import { basename } from "../lib/paths";
import { editorTheme, highlightStyle } from "./theme";

/** Files larger than this open without syntax highlighting, to stay responsive. */
export const MAX_HIGHLIGHT_CHARS = 2 * 1024 * 1024;

const language = new Compartment();

// Deliberately no autocompletion, linting or language intelligence: syntax
// highlighting only (Phase 2). Search is CodeMirror's panel: Cmd+F, Cmd+G, Cmd+Alt+F.
const extensions: Extension = [
  lineNumbers(),
  highlightActiveLineGutter(),
  highlightSpecialChars(),
  history(),
  foldGutter(),
  drawSelection(),
  dropCursor(),
  EditorState.allowMultipleSelections.of(true),
  indentOnInput(),
  syntaxHighlighting(highlightStyle),
  bracketMatching(),
  rectangularSelection(),
  crosshairCursor(),
  highlightActiveLine(),
  highlightSelectionMatches(),
  search({ top: true }),
  keymap.of([...defaultKeymap, ...searchKeymap, ...historyKeymap, ...foldKeymap, indentWithTab]),
  editorTheme,
];

/**
 * The editor state for a file's text (read-only for text that is not a workspace
 * file, such as an agent's). Each tab owns one, so undo history,
 * selection and folds survive switching tabs. Windows line endings are kept: a
 * file whose first line break is CRLF is edited and saved with CRLF.
 */
export function createEditorState(text: string, { readOnly = false } = {}): EditorState {
  const firstBreak = text.indexOf("\n");
  const crlf = firstBreak > 0 && text[firstBreak - 1] === "\r";
  return EditorState.create({
    doc: text,
    extensions: [
      extensions,
      language.of([]),
      crlf ? EditorState.lineSeparator.of("\r\n") : [],
      readOnly ? EditorState.readOnly.of(true) : [],
    ],
  });
}

/** The document's full text, with its own line separator. */
export function textOf(state: EditorState): string {
  return state.sliceDoc();
}

/** Whether the state still needs a language loaded for `path`. */
export function needsLanguage(state: EditorState, path: string): boolean {
  return (
    state.doc.length <= MAX_HIGHLIGHT_CHARS &&
    isEmpty(language.get(state)) &&
    LanguageDescription.matchFilename(languages, basename(path)) !== null
  );
}

/**
 * Loads syntax highlighting for `path`, fetching the grammar on first use. Grammars
 * are separate chunks, so only the languages actually opened are ever loaded.
 */
export async function loadLanguage(path: string): Promise<Extension | null> {
  const description = LanguageDescription.matchFilename(languages, basename(path));
  if (!description) return null;
  return description.support ?? (await description.load());
}

export function setLanguage(support: Extension) {
  return language.reconfigure(support);
}

function isEmpty(extension: Extension | undefined): boolean {
  return extension === undefined || (Array.isArray(extension) && extension.length === 0);
}
