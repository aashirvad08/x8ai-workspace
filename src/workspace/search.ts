import type { SearchEvent } from "../contracts/generated/SearchEvent";
import type { SearchMatch } from "../contracts/generated/SearchMatch";
import type { SearchSummary } from "../contracts/generated/SearchSummary";
import { Store } from "../lib/store";
import type { WorkspaceApi } from "../native";

export interface FileMatches {
  readonly path: string;
  readonly matches: readonly SearchMatch[];
}

export interface SearchSnapshot {
  readonly text: string;
  readonly caseSensitive: boolean;
  readonly status: "idle" | "searching" | "done" | "failed";
  /** In the order the files were found. */
  readonly results: readonly FileMatches[];
  /** Identifies the search the results belong to. */
  readonly run: number;
  /** Set once the search is over. */
  readonly summary: SearchSummary | null;
  readonly error: string | null;
  /** Increases when the search field should take keyboard focus. */
  readonly focusRequest: number;
}

type SearchNative = Pick<WorkspaceApi, "search" | "cancelSearch">;

/** How long typing must pause before the search runs. */
export const SEARCH_DELAY_MS = 250;

/**
 * Plain-text search across the workspace. The native side does the searching
 * and streams results; this keeps the query and what has arrived so far. Only
 * the latest search's results are ever shown.
 */
export class Search extends Store<SearchSnapshot> {
  readonly #native: SearchNative;
  #generation = 0;
  #timer: ReturnType<typeof setTimeout> | undefined;

  constructor(native: SearchNative) {
    super({ text: "", caseSensitive: false, status: "idle", run: 0, results: [], summary: null, error: null, focusRequest: 0 });
    this.#native = native;
  }

  /** Changes the text and searches once typing pauses. */
  setText(text: string): void {
    if (text === this.get().text) return;
    this.update((s) => ({ ...s, text }));
    clearTimeout(this.#timer);
    this.#timer = setTimeout(() => void this.run(), SEARCH_DELAY_MS);
  }

  setCaseSensitive(caseSensitive: boolean): void {
    if (caseSensitive === this.get().caseSensitive) return;
    this.update((s) => ({ ...s, caseSensitive }));
    void this.run();
  }

  requestFocus(): void {
    this.update((s) => ({ ...s, focusRequest: s.focusRequest + 1 }));
  }

  /** Searches now, replacing any running search. Resolves when it is over. */
  async run(): Promise<void> {
    clearTimeout(this.#timer);
    const generation = ++this.#generation;
    const { text, caseSensitive } = this.get();
    if (text === "") {
      this.#clear();
      return;
    }
    this.update((s) => ({ ...s, status: "searching", run: generation, results: [], summary: null, error: null }));

    // Files arrive quickly; they are shown in batches rather than one update each.
    let pending: FileMatches[] = [];
    let flush: ReturnType<typeof setTimeout> | undefined;
    const deliver = () => {
      flush = undefined;
      if (generation !== this.#generation || pending.length === 0) return;
      const batch = pending;
      pending = [];
      this.update((s) => ({ ...s, results: [...s.results, ...batch] }));
    };
    const listener = (event: SearchEvent) => {
      if (generation !== this.#generation) return;
      if (event.type === "file") {
        pending.push({ path: event.path, matches: event.matches });
        flush ??= setTimeout(deliver, 16);
      } else {
        clearTimeout(flush);
        deliver();
        const { type: _, ...summary } = event;
        this.update((s) => ({ ...s, status: "done", summary }));
      }
    };

    try {
      await this.#native.search({ text, caseSensitive }, listener);
    } catch (error) {
      if (generation !== this.#generation) return;
      clearTimeout(flush);
      const message = error instanceof Error ? error.message : String(error);
      this.update((s) => ({ ...s, status: "failed", error: message }));
    }
  }

  /** The workspace changed: old results no longer apply. The text is kept and searched again. */
  reset(): void {
    this.#clear();
    if (this.get().text !== "") void this.run();
  }

  #clear(): void {
    clearTimeout(this.#timer);
    this.#generation++;
    if (this.get().status === "searching") this.#native.cancelSearch().catch(() => {});
    this.update((s) => ({ ...s, status: "idle", results: [], summary: null, error: null }));
  }
}
