import { Store } from "../lib/store";

/**
 * The context composer (docs/multi-agent.md): which sessions give context, which
 * one receives it, what of each, and the exact text that will be pasted into
 * the receiving agent's input. `null` while it is closed.
 */
export type ShareMode = "get" | "give";

export interface ShareParts {
  /** Branch, files changed with their line counts, commits. */
  readonly changes: boolean;
  /** The diff itself, cut at a size limit. */
  readonly diff: boolean;
  /** The last lines its terminal shows. */
  readonly output: boolean;
}

export interface ShareState {
  readonly mode: ShareMode;
  /** The sessions that give context, by id. */
  readonly sources: readonly number[];
  /** The session that receives it. */
  readonly target: number | null;
  readonly parts: ShareParts;
  readonly note: string;
  /** What will be sent: composed from the choices, or as the user edited it. */
  readonly text: string;
  /** The user changed the text by hand; choosing again composes it anew. */
  readonly edited: boolean;
  /** Waiting for the receiving agent, which is starting. */
  readonly sending: boolean;
}

export const DEFAULT_PARTS: ShareParts = { changes: true, diff: false, output: true };

export class ContextShare extends Store<ShareState | null> {
  constructor() {
    super(null);
  }

  open(state: ShareState): void {
    this.set(state);
  }

  close(): void {
    this.set(null);
  }

  /** Changes the open composer; nothing if it is closed. */
  change(change: Partial<ShareState>): void {
    this.update((s) => (s ? { ...s, ...change } : s));
  }
}
