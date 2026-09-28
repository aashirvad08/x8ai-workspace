import type { SessionId } from "../contracts/generated/SessionId";
import type { TerminalInfo } from "../contracts/generated/TerminalInfo";
import { Store } from "../lib/store";
import { type PaneTree, pane, paneKeys, removePane, resizeSplit, type SplitDirection, splitPane } from "./panes";

/** One terminal: a view and the native session it owns. */
export interface TerminalPane {
  /** Identifies the pane in the UI; the native session changes on restart. */
  readonly key: number;
  readonly title: string;
  readonly running: boolean;
  /** The live native session, while `running`. */
  readonly session: SessionId | null;
}

/** A terminal tab: one or more panes, split side by side or one above the other. */
export interface TerminalTab {
  readonly key: number;
  readonly tree: PaneTree;
  /** The pane that has, or last had, keyboard focus. */
  readonly focused: number;
}

export interface TerminalsSnapshot {
  readonly tabs: readonly TerminalTab[];
  readonly panes: ReadonlyMap<number, TerminalPane>;
  readonly active: number | null;
  /** Increases when the active tab's focused pane should take keyboard focus. */
  readonly focusRequest: number;
}

/**
 * Terminal tabs and their split panes. Each pane's view owns its native session,
 * so every pane has its own shell, process and working directory. New sessions
 * start in the workspace root (decided natively); existing ones stay where they
 * are when the workspace changes.
 *
 * Removing a pane here ends its session (its view unmounts). Whether that needs
 * confirming is the workbench's decision, which asks the native side.
 */
export class Terminals extends Store<TerminalsSnapshot> {
  #nextKey = 1;

  constructor() {
    super({ tabs: [], panes: new Map(), active: null, focusRequest: 0 });
  }

  /** Opens a new tab with one pane. Returns the tab's key. */
  add(): number {
    const tab = this.#nextKey++;
    const first = this.#nextKey++;
    this.update((s) => ({
      tabs: [...s.tabs, { key: tab, tree: pane(first), focused: first }],
      panes: withPane(s.panes, first),
      active: tab,
      focusRequest: s.focusRequest + 1,
    }));
    return tab;
  }

  /** Splits the active tab's focused pane; the new pane takes focus. Opens a tab if there is none. */
  split(direction: SplitDirection): void {
    const tab = this.activeTab();
    if (!tab) {
      this.add();
      return;
    }
    const added = this.#nextKey++;
    const id = this.#nextKey++;
    this.#patchTab(tab.key, (t) => ({ ...t, tree: splitPane(t.tree, t.focused, added, direction, id), focused: added }), (s) => ({
      panes: withPane(s.panes, added),
      focusRequest: s.focusRequest + 1,
    }));
  }

  /** Removes a pane, and its tab if it was the last pane there. */
  closePane(key: number): void {
    const tab = this.tabOf(key);
    if (!tab) return;
    const tree = removePane(tab.tree, key);
    if (tree === null) {
      this.closeTab(tab.key);
      return;
    }
    const keys = paneKeys(tab.tree);
    const index = keys.indexOf(key);
    const remaining = paneKeys(tree);
    // Focus moves to the neighbour, as closing a tab does.
    const focused = tab.focused === key ? remaining[Math.min(index, remaining.length - 1)]! : tab.focused;
    this.#patchTab(tab.key, (t) => ({ ...t, tree, focused }), (s) => ({
      panes: withoutPanes(s.panes, [key]),
      focusRequest: s.focusRequest + (s.active === tab.key ? 1 : 0),
    }));
  }

  closeTab(key: number): void {
    this.update((s) => {
      const index = s.tabs.findIndex((tab) => tab.key === key);
      if (index < 0) return s;
      const tabs = s.tabs.filter((tab) => tab.key !== key);
      const active = s.active === key ? (tabs[Math.min(index, tabs.length - 1)]?.key ?? null) : s.active;
      return { ...s, tabs, active, panes: withoutPanes(s.panes, paneKeys(s.tabs[index]!.tree)) };
    });
  }

  activate(key: number): void {
    this.update((s) => (s.active === key ? s : { ...s, active: key }));
  }

  /** Makes `key` its tab's focused pane, and that tab the active one. */
  focusPane(key: number): void {
    const tab = this.tabOf(key);
    if (!tab || (tab.focused === key && this.get().active === tab.key)) return;
    this.#patchTab(tab.key, (t) => ({ ...t, focused: key }), () => ({ active: tab.key }));
  }

  /** Moves focus to the next (`1`) or previous (`-1`) pane of the active tab, wrapping around. */
  focusNext(step: 1 | -1): void {
    const tab = this.activeTab();
    if (!tab) return;
    const keys = paneKeys(tab.tree);
    if (keys.length < 2) return;
    const next = keys[(keys.indexOf(tab.focused) + step + keys.length) % keys.length]!;
    this.#patchTab(tab.key, (t) => ({ ...t, focused: next }), (s) => ({ focusRequest: s.focusRequest + 1 }));
  }

  resize(tabKey: number, splitId: number, ratio: number): void {
    this.#patchTab(tabKey, (t) => ({ ...t, tree: resizeSplit(t.tree, splitId, ratio) }));
  }

  requestFocus(): void {
    this.update((s) => ({ ...s, focusRequest: s.focusRequest + 1 }));
  }

  started(key: number, info: TerminalInfo): void {
    this.#patchPane(key, { title: titleOf(info), running: true, session: info.id });
  }

  ended(key: number): void {
    this.#patchPane(key, { running: false, session: null });
  }

  activeTab(): TerminalTab | undefined {
    const { tabs, active } = this.get();
    return tabs.find((tab) => tab.key === active);
  }

  tabOf(paneKey: number): TerminalTab | undefined {
    return this.get().tabs.find((tab) => paneKeys(tab.tree).includes(paneKey));
  }

  /** Every pane that has a live session, optionally only those of one tab. */
  liveSessions(tabKey?: number): SessionId[] {
    const { tabs, panes } = this.get();
    const keys = tabs.filter((tab) => tabKey === undefined || tab.key === tabKey).flatMap((tab) => paneKeys(tab.tree));
    return keys.flatMap((key) => {
      const session = panes.get(key)?.session;
      return session === undefined || session === null ? [] : [session];
    });
  }

  #patchTab(
    key: number,
    change: (tab: TerminalTab) => TerminalTab,
    also: (s: TerminalsSnapshot) => Partial<TerminalsSnapshot> = () => ({}),
  ): void {
    this.update((s) => {
      const tab = s.tabs.find((t) => t.key === key);
      if (!tab) return s;
      const changed = change(tab);
      const extra = also(s);
      if (changed === tab && Object.keys(extra).length === 0) return s;
      return { ...s, ...extra, tabs: s.tabs.map((t) => (t.key === key ? changed : t)) };
    });
  }

  #patchPane(key: number, change: Partial<TerminalPane>): void {
    this.update((s) => {
      const current = s.panes.get(key);
      if (!current) return s;
      const panes = new Map(s.panes);
      panes.set(key, { ...current, ...change });
      return { ...s, panes };
    });
  }
}

/** A tab is labelled by its focused pane, with the number of panes if split. */
export function tabTitle(tab: TerminalTab, panes: ReadonlyMap<number, TerminalPane>): string {
  const title = panes.get(tab.focused)?.title ?? "Terminal";
  const count = paneKeys(tab.tree).length;
  return count > 1 ? `${title} (${count})` : title;
}

/** `zsh — project`, from the shell and the directory it started in. */
export function titleOf({ program, cwd }: TerminalInfo): string {
  const shell = program.slice(program.lastIndexOf("/") + 1);
  const dir = cwd.slice(cwd.lastIndexOf("/") + 1) || "/";
  return `${shell} — ${dir}`;
}

function withPane(panes: ReadonlyMap<number, TerminalPane>, key: number): Map<number, TerminalPane> {
  const next = new Map(panes);
  next.set(key, { key, title: "Terminal", running: false, session: null });
  return next;
}

function withoutPanes(panes: ReadonlyMap<number, TerminalPane>, keys: readonly number[]): Map<number, TerminalPane> {
  const next = new Map(panes);
  for (const key of keys) next.delete(key);
  return next;
}
