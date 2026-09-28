import { EditorView } from "@codemirror/view";
import { useEffect, useRef } from "react";

import { useStore } from "../lib/useStore";
import type { EditorStore } from "./editor-store";
import { loadLanguage, needsLanguage, setLanguage } from "./setup";

/**
 * The CodeMirror view. There is one view; switching tabs swaps in that tab's
 * editor state, which carries its text, undo history and selection. Every change
 * made in the view is handed back to the store.
 */
export function EditorPane({ editor }: { editor: EditorStore }) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const shown = useRef<string | null>(null);
  const revealed = useRef(0);
  const { active, revision, reveal } = useStore(editor);

  useEffect(() => {
    const created = new EditorView({
      parent: host.current!,
      dispatchTransactions: (transactions, target) => {
        target.update(transactions);
        if (shown.current !== null) editor.applyViewState(shown.current, target.state);
      },
    });
    view.current = created;
    return () => {
      created.destroy();
      view.current = null;
    };
  }, [editor]);

  useEffect(() => {
    const current = view.current;
    const state = active === null ? undefined : editor.stateOf(active);
    if (!current || active === null || !state) {
      shown.current = null;
      return;
    }
    const switched = shown.current !== active;
    shown.current = active;
    if (current.state !== state) current.setState(state);
    if (reveal !== revealed.current) {
      revealed.current = reveal;
      current.dispatch({ effects: EditorView.scrollIntoView(current.state.selection.main, { y: "center" }) });
      current.focus();
    } else if (switched) {
      current.focus();
    }

    // Syntax highlighting is loaded on first display, then kept in the tab's state.
    if (needsLanguage(state, active)) {
      loadLanguage(active)
        .then((support) => {
          const target = view.current;
          if (support && target && shown.current === active && needsLanguage(target.state, active)) {
            target.dispatch({ effects: setLanguage(support) });
          }
        })
        .catch((error: unknown) => console.warn(`No syntax highlighting for ${active}`, error));
    }
  }, [editor, active, revision, reveal]);

  return <div className="editor-pane" ref={host} hidden={active === null} />;
}
