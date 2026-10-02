import { memo } from "react";

import { useStore } from "../lib/useStore";
import type { EditorActions } from "./actions";
import type { EditorStore, TabInfo } from "./editor-store";
import { EditorPane } from "./EditorPane";

export const EditorArea = memo(function EditorArea({ editor, actions }: { editor: EditorStore; actions: EditorActions }) {
  const { tabs, active } = useStore(editor);
  const current = tabs.find((tab) => tab.path === active);

  return (
    <section className="editor-area" aria-label="Editor">
      {tabs.length > 0 && (
        <div className="tab-bar" role="tablist">
          {tabs.map((tab) => (
            <Tab key={tab.path} tab={tab} active={tab.path === active} editor={editor} actions={actions} />
          ))}
        </div>
      )}
      {current?.disk === "changed" && (
        <div className="banner" role="status">
          “{current.name}” changed on disk while you were editing it.
          <button type="button" onClick={() => actions.revert(current.path)}>
            Revert to Disk
          </button>
          <button type="button" onClick={() => actions.overwrite(current.path)}>
            Overwrite with Mine
          </button>
        </div>
      )}
      {current?.disk === "deleted" && (
        <div className="banner" role="status">
          “{current.name}” was deleted on disk. Saving will recreate it.
          <button type="button" onClick={() => actions.overwrite(current.path)}>
            Save
          </button>
        </div>
      )}
      <EditorPane editor={editor} />
      {tabs.length === 0 && (
        <div className="editor-empty">
          <p>
            <kbd>⌘P</kbd> Go to File <kbd>⇧⌘P</kbd> Commands <kbd>⌃`</kbd> Terminal
          </p>
        </div>
      )}
    </section>
  );
});

function Tab({
  tab,
  active,
  editor,
  actions,
}: {
  tab: TabInfo;
  active: boolean;
  editor: EditorStore;
  actions: EditorActions;
}) {
  const classes = ["tab", active && "tab-active", tab.dirty && "tab-dirty", tab.disk !== "synced" && "tab-stale"];
  // The close button is a sibling of the tab, not inside it: interactive content
  // inside a tab is invalid, and assistive technology pressing the tab could hit it.
  return (
    <div
      role="presentation"
      className={classes.filter(Boolean).join(" ")}
      title={tab.title + (tab.dirty ? " (unsaved)" : tab.readOnly ? " (read-only)" : "")}
      onAuxClick={(event) => event.button === 1 && actions.close(tab.path)}
    >
      <button type="button" role="tab" aria-selected={active} className="tab-name" onClick={() => editor.activate(tab.path)}>
        {tab.name}
      </button>
      <button
        type="button"
        className="tab-close"
        aria-label={tab.dirty ? `Close ${tab.name} (unsaved)` : `Close ${tab.name}`}
        onClick={() => actions.close(tab.path)}
      >
        <span className="tab-dot" aria-hidden>
          ●
        </span>
        <span className="tab-x" aria-hidden>
          ×
        </span>
      </button>
    </div>
  );
}
