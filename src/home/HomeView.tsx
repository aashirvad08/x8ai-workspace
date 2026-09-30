import { type FormEvent, type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";

import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import type { Store } from "../lib/store";
import { useStore } from "../lib/useStore";
import type { HomeActions } from "./actions";
import { type Home, suggestionsFor } from "./home";
import { GLYPH_HEIGHT, pixelate } from "./pixelFont";

interface Props {
  home: Home;
  recent: Store<readonly RecentWorkspace[]>;
  workspace: Store<WorkspaceInfo | null>;
  actions: HomeActions;
}

/**
 * The head of the app (⇧⌘H): a greeting and a command line. `/cd` opens a space,
 * `/home` the workspace with no folder; Esc goes back to the space as it is.
 */
export function HomeView({ home, recent: recentStore, workspace: workspaceStore, actions }: Props) {
  const { message } = useStore(home);
  const recent = useStore(recentStore);
  const workspace = useStore(workspaceStore);
  const name = home.name();
  const [text, setText] = useState("");
  const [selected, setSelected] = useState(0);
  const [busy, setBusy] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const suggestions = useMemo(() => suggestionsFor(text, recent), [text, recent]);

  useEffect(() => input.current?.focus(), []);
  useEffect(() => setSelected(0), [text]);

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
  const submit = (event: FormEvent) => {
    event.preventDefault();
    if (!busy) void run(text);
  };
  const onKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    const suggestion = suggestions[selected];
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

  const spaces = recent.filter((r) => r.available && r.root !== workspace?.root).slice(0, 5);
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
              onChange={(e) => setText(e.target.value)}
              onKeyDown={onKeyDown}
            />
          </div>
          <div className="home-context">
            {workspace ? (
              <>
                <span className="home-context-kind">Space</span>
                <span className="home-context-name">{workspace.name}</span>
                <span className="home-context-detail" title={workspace.root}>
                  {workspace.trusted ? "trusted" : "not trusted"}
                </span>
              </>
            ) : (
              <>
                <span className="home-context-kind">Home</span>
                <span className="home-context-name">no folder open</span>
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
        {message && (
          <p className={`home-message home-message-${message.tone}`} role={message.tone === "error" ? "alert" : "status"}>
            {message.text}
          </p>
        )}
        <p className="home-hints">
          <Hint keys="/cd" text="open a space" />
          <Hint keys="/home" text="no folder" />
          <Hint keys="/name" text="your name" />
          <Hint keys="esc" text={workspace ? `back to ${workspace.name}` : "to the workspace"} />
        </p>
        {spaces.length > 0 && (
          <div className="home-recent">
            <span className="home-recent-title">Recent spaces</span>
            <ul>
              {spaces.map((r) => (
                <li key={r.root}>
                  <button type="button" className="home-recent-item" title={r.root} disabled={busy} onClick={() => void run(`/cd ${r.root}`)}>
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
}

function Hint({ keys, text }: { keys: string; text: string }) {
  return (
    <span className="home-hint">
      <kbd>{keys}</kbd> {text}
    </span>
  );
}

/** "WELCOME, SIR" in pixels, the name under it; plain text where a letter has no glyph. */
function Headline({ name }: { name: string | null }) {
  const welcome = pixelate("Welcome,");
  const sir = welcome && pixelate("Sir", welcome.width + 4);
  const named = name ? pixelate(name) : null;
  const title = name ? `Welcome, Sir ${name}` : "Welcome, Sir";
  if (!welcome || !sir) return <h1 className="home-title-text">{title}</h1>;
  const width = welcome.width + 4 + sir.width;
  return (
    <h1 className="home-title" aria-label={title}>
      <Pixels width={width} runs={[{ run: welcome, tone: "dim" }, { run: sir, tone: "bright" }]} className="home-pixels" />
      {name &&
        (named && named.width <= width * 1.6 ? (
          <Pixels width={named.width} runs={[{ run: named, tone: "accent" }]} className="home-pixels-name" />
        ) : (
          <span className="home-name-text">{name}</span>
        ))}
    </h1>
  );
}

function Pixels({
  width,
  runs,
  className,
}: {
  width: number;
  runs: readonly { run: NonNullable<ReturnType<typeof pixelate>>; tone: "dim" | "bright" | "accent" }[];
  className: string;
}) {
  // Each pixel is a slightly inset square with its shadow one step down and right.
  const height = GLYPH_HEIGHT;
  return (
    <svg
      className={className}
      viewBox={`-0.1 -0.1 ${width + 0.5} ${height + 0.5}`}
      style={{ aspectRatio: `${width + 0.5} / ${height + 0.5}` }}
      shapeRendering="crispEdges"
      aria-hidden
    >
      {runs.map(({ run, tone }) =>
        run.pixels.map(([x, y]) => (
          <rect key={`s${tone}${x},${y}`} className={`pixel-shadow-${tone}`} x={x + 0.22} y={y + 0.22} width={0.88} height={0.88} />
        )),
      )}
      {runs.map(({ run, tone }) =>
        run.pixels.map(([x, y]) => (
          <rect key={`p${tone}${x},${y}`} className={`pixel-${tone}`} x={x} y={y} width={0.88} height={0.88} />
        )),
      )}
    </svg>
  );
}
