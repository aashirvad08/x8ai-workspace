import { useStore } from "../lib/useStore";
import type { TerminalApi } from "../native";
import type { Terminals } from "./terminals";
import { TerminalView } from "./TerminalView";

interface Props {
  native: TerminalApi;
  terminals: Terminals;
  /** Hidden panels keep their sessions running. */
  hidden: boolean;
  onHide: () => void;
}

export function TerminalPanel({ native, terminals, hidden, onHide }: Props) {
  const { tabs, active, focusRequest } = useStore(terminals);

  return (
    <section className="terminal-panel" aria-label="Terminal" hidden={hidden}>
      <header className="panel-header">
        <div className="panel-tabs" role="tablist">
          {tabs.map((tab) => (
            // The close button is a sibling of the tab, not inside it (see EditorArea).
            <div key={tab.key} role="presentation" className={tab.key === active ? "panel-tab panel-tab-active" : "panel-tab"}>
              <button
                type="button"
                role="tab"
                aria-selected={tab.key === active}
                className={tab.running ? "panel-tab-title" : "panel-tab-title panel-tab-ended"}
                onClick={() => {
                  terminals.activate(tab.key);
                  terminals.requestFocus();
                }}
              >
                {tab.title}
              </button>
              <button
                type="button"
                className="panel-tab-close"
                aria-label={`Close ${tab.title}`}
                title="Close terminal (ends its processes)"
                onClick={() => terminals.close(tab.key)}
              >
                ×
              </button>
            </div>
          ))}
          <button type="button" className="icon-button" title="New Terminal (⌃⇧`)" aria-label="New terminal" onClick={() => terminals.add()}>
            +
          </button>
        </div>
        <button type="button" className="icon-button" title="Hide Terminal (⌃`)" aria-label="Hide terminal" onClick={onHide}>
          ⌄
        </button>
      </header>
      <div className="panel-body">
        {tabs.map((tab) => (
          <TerminalView
            key={tab.key}
            native={native}
            active={tab.key === active}
            focusRequest={focusRequest}
            onStart={(info) => terminals.started(tab.key, info)}
            onEnd={() => terminals.ended(tab.key)}
          />
        ))}
        {tabs.length === 0 && (
          <div className="panel-empty">
            <button type="button" className="button-primary" onClick={() => terminals.add()}>
              New Terminal
            </button>
          </div>
        )}
      </div>
    </section>
  );
}
