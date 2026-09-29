import { Store } from "../lib/store";

export type SidebarView = "files" | "search" | "agents" | "models" | "mcp";

export interface LayoutState {
  readonly explorerWidth: number;
  readonly terminalHeight: number;
  readonly explorerVisible: boolean;
  readonly terminalVisible: boolean;
  /** What the sidebar (the explorer's column) shows. */
  readonly sidebar: SidebarView;
}

const DEFAULTS: LayoutState = {
  explorerWidth: 260,
  terminalHeight: 280,
  explorerVisible: true,
  terminalVisible: true,
  sidebar: "files",
};
const STORAGE_KEY = "x8ai.layout";

export const EXPLORER_WIDTH = { min: 160, max: 600 };
export const TERMINAL_HEIGHT = { min: 100, max: 2000 };

/**
 * Panel sizes and visibility. Remembered per machine in localStorage, a
 * convenience only: if storage is unavailable, the defaults apply.
 */
export class Layout extends Store<LayoutState> {
  constructor() {
    super(load());
  }

  resizeExplorer(width: number): void {
    this.#change({ explorerWidth: clamp(width, EXPLORER_WIDTH) });
  }

  resizeTerminal(height: number): void {
    this.#change({ terminalHeight: clamp(height, TERMINAL_HEIGHT) });
  }

  toggleExplorer(): void {
    this.#change({ explorerVisible: !this.get().explorerVisible });
  }

  /** Shows the sidebar with `view` in it. */
  showSidebar(sidebar: SidebarView): void {
    this.#change({ sidebar, explorerVisible: true });
  }

  setTerminalVisible(terminalVisible: boolean): void {
    this.#change({ terminalVisible });
  }

  #change(change: Partial<LayoutState>): void {
    this.update((s) => ({ ...s, ...change }));
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(this.get()));
    } catch {
      // Storage unavailable (private mode, tests): the layout is just not remembered.
    }
  }
}

function load(): LayoutState {
  try {
    const stored = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}") as Partial<LayoutState>;
    return {
      explorerWidth: clamp(Number(stored.explorerWidth ?? DEFAULTS.explorerWidth), EXPLORER_WIDTH),
      terminalHeight: clamp(Number(stored.terminalHeight ?? DEFAULTS.terminalHeight), TERMINAL_HEIGHT),
      explorerVisible: stored.explorerVisible ?? DEFAULTS.explorerVisible,
      terminalVisible: stored.terminalVisible ?? DEFAULTS.terminalVisible,
      sidebar: (["search", "agents", "models", "mcp"] as const).find((v) => v === stored.sidebar) ?? "files",
    };
  } catch {
    return DEFAULTS;
  }
}

function clamp(value: number, { min, max }: { min: number; max: number }): number {
  return Number.isFinite(value) ? Math.min(max, Math.max(min, Math.round(value))) : min;
}
