import { HighlightStyle } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags as t } from "@lezer/highlight";

// Colours are CSS variables defined in src/app/app.css, so the editor follows the
// system light or dark appearance without reconfiguring.

export const highlightStyle = HighlightStyle.define([
  { tag: [t.keyword, t.operatorKeyword, t.modifier, t.controlKeyword], color: "var(--syntax-keyword)" },
  { tag: [t.string, t.special(t.string), t.inserted, t.processingInstruction], color: "var(--syntax-string)" },
  { tag: [t.number, t.bool, t.null, t.atom, t.unit], color: "var(--syntax-number)" },
  { tag: [t.comment, t.meta, t.documentMeta], color: "var(--syntax-comment)", fontStyle: "italic" },
  { tag: [t.function(t.variableName), t.function(t.propertyName), t.labelName], color: "var(--syntax-function)" },
  { tag: [t.typeName, t.className, t.namespace, t.annotation, t.self], color: "var(--syntax-type)" },
  { tag: [t.propertyName, t.attributeName], color: "var(--syntax-property)" },
  { tag: [t.constant(t.name), t.standard(t.name), t.color], color: "var(--syntax-constant)" },
  { tag: [t.operator, t.regexp, t.escape, t.url], color: "var(--syntax-operator)" },
  { tag: [t.tagName, t.heading], color: "var(--syntax-tag)", fontWeight: "600" },
  { tag: t.strong, fontWeight: "bold" },
  { tag: t.emphasis, fontStyle: "italic" },
  { tag: t.strikethrough, textDecoration: "line-through" },
  { tag: t.link, color: "var(--syntax-operator)", textDecoration: "underline" },
  { tag: [t.deleted, t.invalid], color: "var(--syntax-invalid)" },
]);

export const editorTheme = EditorView.theme({
  "&": { height: "100%", color: "var(--text)", backgroundColor: "var(--bg)", fontSize: "13px" },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-code)", lineHeight: "1.55" },
  ".cm-content": { caretColor: "var(--accent)", padding: "6px 0" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)", borderLeftWidth: "2px" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, .cm-content ::selection":
    { backgroundColor: "var(--selection)" },
  ".cm-gutters": { backgroundColor: "var(--bg)", color: "var(--text-faint)", border: "none" },
  ".cm-activeLine": { backgroundColor: "var(--active-line)" },
  ".cm-activeLineGutter": { backgroundColor: "var(--active-line)", color: "var(--text-muted)" },
  ".cm-foldPlaceholder": { backgroundColor: "var(--bg-raised)", border: "none", color: "var(--text-muted)" },
  ".cm-matchingBracket": { backgroundColor: "var(--match)", outline: "1px solid var(--border-strong)" },
  ".cm-searchMatch": { backgroundColor: "var(--match)" },
  ".cm-searchMatch.cm-searchMatch-selected": { backgroundColor: "var(--match-selected)" },
  ".cm-selectionMatch": { backgroundColor: "var(--match)" },
  ".cm-panels": { backgroundColor: "var(--bg-raised)", color: "var(--text)" },
  ".cm-panels.cm-panels-top": { borderBottom: "1px solid var(--border)" },
  ".cm-panel.cm-search": { fontFamily: "var(--font-ui)", fontSize: "12px", padding: "6px 8px" },
  ".cm-panel.cm-search input, .cm-panel.cm-search button": { fontFamily: "var(--font-ui)", fontSize: "12px" },
  ".cm-textfield": {
    backgroundColor: "var(--bg)",
    color: "var(--text)",
    border: "1px solid var(--border-strong)",
    borderRadius: "4px",
  },
  ".cm-button": {
    backgroundImage: "none",
    backgroundColor: "var(--bg)",
    color: "var(--text)",
    border: "1px solid var(--border-strong)",
    borderRadius: "4px",
  },
});
