import { memo, type FormEvent, type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";

import { type Addons, matchSpaces } from "../addons/addons";
import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import type { SpaceInfo } from "../contracts/generated/SpaceInfo";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import type { Store } from "../lib/store";
import { useStore } from "../lib/useStore";
import type { HomeActions } from "./actions";
import { type Home, suggestionsFor } from "./home";

interface Props {
  home: Home;
  recent: Store<readonly RecentWorkspace[]>;
  workspace: Store<WorkspaceInfo | null>;
  spaces: Store<readonly SpaceInfo[]>;
  addons: Addons;
  actions: HomeActions;
}

/** The space `/share <arg>` names, when it names exactly one other. */
function shareTarget(text: string, spaces: readonly SpaceInfo[], open: string | null): SpaceInfo | null {
  const match = /^\/share\s+(.+)$/.exec(text);
  if (!match) return null;
  const found = matchSpaces(match[1]!, spaces, open);
  return found.length === 1 ? found[0]! : null;
}

/**
 * The head of the app (⇧⌘H): a greeting and a command line. `/cd` opens a space,
 * `/new` makes one, `/share` gives another space this one's add-ons, `/home`
 * the workspace with no folder; Esc goes back to the space as it is.
 */
export const HomeView = memo(function HomeView({
  home,
  recent: recentStore,
  workspace: workspaceStore,
  spaces: spacesStore,
  addons,
  actions,
}: Props) {
  const { message, draft } = useStore(home);
  const recent = useStore(recentStore);
  const workspace = useStore(workspaceStore);
  const spaces = useStore(spacesStore);
  const { list } = useStore(addons);
  const name = home.name();
  const [text, setText] = useState("");
  const [selected, setSelected] = useState(0);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const openSpace = list?.space.id ?? null;
  const target = useMemo(() => shareTarget(text, spaces, openSpace), [text, spaces, openSpace]);
  const suggestions = useMemo(
    () => (target ? [] : suggestionsFor(text, recent, spaces, openSpace)),
    [target, text, recent, spaces, openSpace],
  );
  // `/share`: the open space's add-ons, every one given unless unchecked; ↓ then
  // Space chooses, Enter shares.
  const shareable = useMemo(() => list?.addons.filter((a) => a.added) ?? [], [list]);
  const [unchecked, setUnchecked] = useState<ReadonlySet<string>>(new Set());
  const [highlight, setHighlight] = useState(-1);
  const chosen = shareable.filter((a) => !unchecked.has(a.id));
  const toggle = (id: string) =>
    setUnchecked((was) => {
      const next = new Set(was);
      if (!next.delete(id)) next.add(id);
      return next;
    });

  useEffect(() => input.current?.focus(), []);
  useEffect(() => setSelected(0), [text]);
  useEffect(() => {
    setUnchecked(new Set());
    setHighlight(-1);
  }, [target?.id]);
  useEffect(() => {
    if (draft === null) return;
    const taken = home.takeDraft();
    if (taken !== null) setText(taken);
    input.current?.focus();
  }, [draft, home]);

  const run = async (command: string) => {
    setBusy(true);
    try {
      await actions.runHomeCommand(command);
    } finally {
      setBusy(false);
      setText("");
      input.current?.focus();
    }
  };
  const share = async (to: string) => {
    setBusy(true);
    try {
      if (await actions.shareSpace(to, chosen.map((a) => a.id))) setText("");
    } finally {
      setBusy(false);
      input.current?.focus();
    }
  };
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (busy) return;
    if (target) void share(target.id);
    else void run(text);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const suggestion = suggestions[selected];
    if (target && shareable.length > 0) {
      if (event.key === "ArrowDown") {
        event.preventDefault();
        setHighlight(Math.min(highlight + 1, shareable.length - 1));
        return;
      }
      if (event.key === "ArrowUp") {
        event.preventDefault();
        setHighlight(Math.max(highlight - 1, -1));
        return;
      }
      if (event.key === " " && highlight >= 0) {
        event.preventDefault();
        toggle(shareable[highlight]!.id);
        return;
      }
    }
    if (event.key === "Escape") {
      event.preventDefault();
      if (text) setText("");
      else actions.leaveHome();
    } else if (event.key === "Tab" && suggestion) {
      event.preventDefault();
      setText(suggestion.completion);
    } else if (event.key === "ArrowDown" && suggestions.length > 0) {
      event.preventDefault();
      setSelected((selected + 1) % suggestions.length);
    } else if (event.key === "ArrowUp" && suggestions.length > 0) {
      event.preventDefault();
      setSelected((selected - 1 + suggestions.length) % suggestions.length);
    }
  };

  const others = recent.filter((r) => r.available && r.root !== workspace?.root).slice(0, 5);
  return (
    <section className="home" aria-label="Welcome">
      <div className="home-center">
        <Headline name={name} />
        <form className="home-prompt" onSubmit={submit}>
          <div className="home-line">
            <span className="home-caret" aria-hidden>
              ›
            </span>
            <input
              ref={input}
              className="home-input"
              value={text}
              disabled={busy}
              spellCheck={false}
              autoComplete="off"
              aria-label="Command"
              placeholder='Type a command…  "/cd ~/projects/app"'
              onChange={(e) => {
                setText(e.target.value);
                setHighlight(-1);
              }}
              onKeyDown={onKeyDown}
            />
          </div>
          <div className="home-context">
            {workspace ? (
              <>
                <span className="home-context-kind">Space</span>
                <span className="home-context-name">{workspace.name}</span>
                <span className="home-context-detail" title={workspace.root}>
                  {workspace.id} · {workspace.trusted ? "trusted" : "not trusted"}
                </span>
              </>
            ) : (
              <>
                <span className="home-context-kind">Home</span>
                <span className="home-context-name">no folder open</span>
                {openSpace && <span className="home-context-detail">{openSpace}</span>}
              </>
            )}
          </div>
        </form>
        {suggestions.length > 0 && (
          <ul className="home-suggestions" aria-label="Suggestions">
            {suggestions.map((s, i) => (
              <li key={s.completion}>
                <button
                  type="button"
                  className={i === selected ? "home-suggestion home-suggestion-selected" : "home-suggestion"}
                  onMouseEnter={() => setSelected(i)}
                  onClick={() => {
                    setText(s.completion);
                    input.current?.focus();
                  }}
                >
                  <span className="home-suggestion-label">{s.label}</span>
                  <span className="home-suggestion-detail">{s.detail}</span>
                </button>
              </li>
            ))}
          </ul>
        )}
        {target && (
          <div className="home-share" aria-label={`Share with ${target.name}`}>
            <p className="home-share-title">
              Share with <span className="home-share-target">{target.name}</span>{" "}
              <span className="home-share-id">{target.id}</span>
            </p>
            {shareable.length === 0 ? (
              <p className="home-share-empty">This space has no add-ons yet. Add some in Add-ons (⇧⌘X) first.</p>
            ) : (
              <>
                <ul className="home-share-list" role="listbox" aria-multiselectable aria-label="Add-ons to share">
                  {shareable.map((addon, i) => {
                    const there = target.addons.includes(addon.id);
                    return (
                      <li key={addon.id} role="option" aria-selected={!unchecked.has(addon.id)}>
                        <label className={i === highlight ? "home-share-item home-share-item-highlight" : "home-share-item"} onMouseEnter={() => setHighlight(i)}>
                          <input type="checkbox" checked={!unchecked.has(addon.id)} onChange={() => toggle(addon.id)} tabIndex={-1} />
                          <span>{addon.name}</span>
                          {there && <span className="home-share-there">has it</span>}
                        </label>
                      </li>
                    );
                  })}
                </ul>
                <p className="home-share-help">
                  <kbd>↵</kbd> shares {chosen.length === shareable.length ? "all" : `${chosen.length} of ${shareable.length}`} · <kbd>↓</kbd> then{" "}
                  <kbd>space</kbd> to choose
                </p>
              </>
            )}
          </div>
        )}
        {message && (
          <p className={`home-message home-message-${message.tone}`} role={message.tone === "error" ? "alert" : "status"}>
            {message.text}
          </p>
        )}
        <p className="home-hints">
          <Hint keys="/cd" text="open a space" />
          <Hint keys="/new" text="new space" />
          <Hint keys="/share" text="share add-ons" />
          <Hint keys="/home" text="no folder" />
          <Hint keys="esc" text={workspace ? `back to ${workspace.name}` : "to the workspace"} />
        </p>
        {others.length > 0 && (
          <div className="home-recent">
            <span className="home-recent-title">Recent spaces</span>
            <ul>
              {others.map((r) => (
                <li key={r.root}>
                  <button type="button" className="home-recent-item" title={r.id ? `${r.root} · ${r.id}` : r.root} disabled={busy} onClick={() => void run(`/cd ${r.root}`)}>
                    <span className="home-recent-name">{r.name}</span>
                    <span className="home-recent-path">{r.root}</span>
                  </button>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </section>
  );
});

function Hint({ keys, text }: { keys: string; text: string }) {
  return (
    <span className="home-hint">
      <kbd>{keys}</kbd> {text}
    </span>
  );
}

/** "Welcome, Sir", a terminal cursor after it, and the name under it. */
function Headline({ name }: { name: string | null }) {
  return (
    <h1 className="home-title" aria-label={name ? `Welcome, Sir ${name}` : "Welcome, Sir"}>
      <span className="home-greeting" aria-hidden>
        <span className="home-greeting-dim">Welcome,</span>
        <span className="home-greeting-sir">Sir</span>
        <span className="home-cursor" />
      </span>
      {name && (
        <span className="home-name" aria-hidden>
          {name}
        </span>
      )}
    </h1>
  );
}
