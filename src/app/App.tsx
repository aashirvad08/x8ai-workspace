import type { ReactNode } from "react";

import type { NativeClient } from "../native";
import { StatusBar } from "./StatusBar";
import { useNativeStatus } from "./useNativeStatus";
import "./app.css";

/**
 * The workspace shell. Each region is an honest placeholder until its phase lands
 * (docs/roadmap.md). The terminal is the primary region by design: agents run
 * inside terminal sessions rather than in a separate chat panel.
 */
export function App({ native }: { native: NativeClient }) {
  const status = useNativeStatus(native);

  return (
    <div className="shell">
      <aside className="sidebar">
        <Placeholder title="Workspace" phase={2}>
          Projects, files and sessions.
        </Placeholder>
      </aside>
      <main className="regions">
        {/* TODO(phase-1): replace with the session view from src/terminal/. */}
        <Placeholder title="Terminal" phase={1}>
          Real PTY sessions running your login shell. Agents such as Claude Code, OpenCode, Codex or Aider run here
          from Phase 4.
        </Placeholder>
        <Placeholder title="Editor" phase={3}>
          Files, tabs and syntax highlighting.
        </Placeholder>
      </main>
      <StatusBar status={status} />
    </div>
  );
}

function Placeholder({ title, phase, children }: { title: string; phase: number; children: ReactNode }) {
  return (
    <section className="placeholder" aria-label={title}>
      <header>
        <h2>{title}</h2>
        <span className="badge">Phase {phase}</span>
      </header>
      <p>{children}</p>
    </section>
  );
}
