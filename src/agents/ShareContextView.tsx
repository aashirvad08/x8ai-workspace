import { useEffect } from "react";

import { useStore } from "../lib/useStore";
import type { ContextActions } from "./actions";
import type { Agents } from "./agents";
import { sessionLabel } from "./context";
import type { ContextShare, ShareParts } from "./share";

const PARTS: readonly { part: keyof ShareParts; label: string; detail: string }[] = [
  { part: "changes", label: "What changed", detail: "branch, files and line counts, commits" },
  { part: "diff", label: "The diff", detail: "the changes themselves, cut at 40 KB" },
  { part: "output", label: "Recent terminal output", detail: "the last lines its terminal shows" },
];

/**
 * The context composer (/get, /give): from which sessions, to which one, what of
 * each, and the exact text, editable, that goes into the receiving agent's
 * input. Nothing is sent until the user presses Enter in that agent.
 */
export function ShareContextView({ share, agents, actions }: { share: ContextShare; agents: Agents; actions: ContextActions }) {
  const state = useStore(share);
  const { sessions } = useStore(agents);

  useEffect(() => {
    if (!state) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") actions.closeShareContext();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [state === null, actions]);

  if (!state) return null;
  const target = sessions.find((s) => s.id === state.target);
  const from = sessions.filter((s) => s.id !== state.target);
  const size = new Blob([state.text]).size;
  const sizeLabel = size < 1024 ? `${size} bytes` : `${(size / 1024).toFixed(1)} KB`;
  const canSend = target !== undefined && state.text.trim() !== "" && !state.sending;

  return (
    <div className="overlay" onPointerDown={(e) => e.target === e.currentTarget && actions.closeShareContext()}>
      <div className="dialog share" role="dialog" aria-label="Share context">
        <h2>{target ? `Context for ${target.name}` : "Share context"}</h2>
        <p>
          Hand another session's work to an agent, so it can carry on without looking through every file. It goes into the
          agent's input: you read it there, and nothing is sent until you press Enter.
        </p>
        <div className="share-columns">
          <fieldset className="share-group">
            <legend>From</legend>
            {from.length === 0 && <p className="share-empty">No other session.</p>}
            {from.map((s) => (
              <label key={s.id} className="share-option">
                <input
                  type="checkbox"
                  checked={state.sources.includes(s.id)}
                  onChange={(e) =>
                    actions.setShareSources(
                      e.target.checked ? [...state.sources, s.id] : state.sources.filter((id) => id !== s.id),
                    )
                  }
                />
                <span>{sessionLabel(s)}</span>
              </label>
            ))}
          </fieldset>
          <fieldset className="share-group">
            <legend>To</legend>
            <select
              className="model-select"
              aria-label="The session that receives the context"
              value={state.target ?? ""}
              onChange={(e) => actions.setShareTarget(e.target.value === "" ? null : Number(e.target.value))}
            >
              {sessions.map((s) => (
                <option key={s.id} value={s.id}>
                  {sessionLabel(s)}
                </option>
              ))}
            </select>
            <span className="share-sublegend">Include</span>
            {PARTS.map(({ part, label, detail }) => (
              <label key={part} className="share-option" title={detail}>
                <input
                  type="checkbox"
                  checked={state.parts[part]}
                  onChange={(e) => actions.setShareParts({ ...state.parts, [part]: e.target.checked })}
                />
                <span>{label}</span>
              </label>
            ))}
          </fieldset>
        </div>
        <label className="share-field">
          Note
          <textarea
            className="model-input"
            rows={2}
            value={state.note}
            placeholder="What it should do next, or what to watch out for"
            onChange={(e) => actions.setShareNote(e.target.value)}
          />
        </label>
        <label className="share-field">
          <span>
            What will be sent ({sizeLabel}){state.edited ? " · edited: choosing again composes it anew" : ""}
          </span>
          <textarea
            className="model-input share-text"
            rows={12}
            spellCheck={false}
            value={state.text}
            onChange={(e) => actions.editShareText(e.target.value)}
          />
        </label>
        <p className="share-warning">
          All of this goes to {target?.name ?? "the agent"} and the model provider it uses. Terminal output can contain
          secrets: check it first.
        </p>
        <div className="dialog-buttons">
          <button type="button" className="button-primary" disabled={!canSend} onClick={() => actions.sendShareContext()}>
            {state.sending ? "Waiting for the agent…" : "Put in its input"}
          </button>
          <button type="button" onClick={() => actions.closeShareContext()}>
            Cancel
          </button>
        </div>
      </div>
    </div>
  );
}
