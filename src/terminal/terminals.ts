import type { TerminalInfo } from "../contracts/generated/TerminalInfo";
import { Store } from "../lib/store";

export interface TerminalTab {
  /** Identifies the tab in the UI; the native session id can change on restart. */
  readonly key: number;
  readonly title: string;
  readonly running: boolean;
}

export interface TerminalsSnapshot {
  readonly tabs: readonly TerminalTab[];
  readonly active: number | null;
  /** Increases when the active terminal should take keyboard focus. */
  readonly focusRequest: number;
}

/**
 * Terminal tabs. Each tab's view owns its native session. New sessions start in
 * the workspace root (decided natively); existing ones stay where they are when
 * the workspace changes.
 */
export class Terminals extends Store<TerminalsSnapshot> {
  #nextKey = 1;

  constructor() {
    super({ tabs: [], active: null, focusRequest: 0 });
  }

  add(): number {
    const key = this.#nextKey++;
    this.update((s) => ({
      ...s,
      tabs: [...s.tabs, { key, title: "Terminal", running: false }],
      active: key,
      focusRequest: s.focusRequest + 1,
    }));
    return key;
  }

  close(key: number): void {
    this.update((s) => {
      const index = s.tabs.findIndex((tab) => tab.key === key);
      if (index < 0) return s;
      const tabs = s.tabs.filter((tab) => tab.key !== key);
      const active = s.active === key ? (tabs[Math.min(index, tabs.length - 1)]?.key ?? null) : s.active;
      return { ...s, tabs, active };
    });
  }

  requestFocus(): void {
    this.update((s) => ({ ...s, focusRequest: s.focusRequest + 1 }));
  }

  activate(key: number): void {
    this.update((s) => (s.active === key ? s : { ...s, active: key }));
  }

  started(key: number, info: TerminalInfo): void {
    this.#patch(key, { title: titleOf(info), running: true });
  }

  ended(key: number): void {
    this.#patch(key, { running: false });
  }

  #patch(key: number, change: Partial<TerminalTab>): void {
    this.update((s) => ({ ...s, tabs: s.tabs.map((tab) => (tab.key === key ? { ...tab, ...change } : tab)) }));
  }
}

/** `zsh — project`, from the shell and the directory it started in. */
export function titleOf({ program, cwd }: TerminalInfo): string {
  const shell = program.slice(program.lastIndexOf("/") + 1);
  const dir = cwd.slice(cwd.lastIndexOf("/") + 1) || "/";
  return `${shell} — ${dir}`;
}
