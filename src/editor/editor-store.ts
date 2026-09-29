import type { EditorState, Text } from "@codemirror/state";

import type { FileVersion } from "../contracts/generated/FileVersion";
import { basename, isWithin, rebase } from "../lib/paths";
import { Store } from "../lib/store";
import type { WorkspaceApi } from "../native";
import { createEditorState, textOf } from "./setup";

/** How the tab relates to the file on disk. */
export type DiskStatus =
  /** Matches the version last read or saved. */
  | "synced"
  /** Changed on disk while the tab had unsaved edits. Saving reports a conflict. */
  | "changed"
  /** Gone from disk. Saving recreates it (after confirming the conflict). */
  | "deleted";

export interface TabInfo {
  /**
   * A workspace path, or for a read-only document a key starting with `/`, which
   * no workspace path can.
   */
  readonly path: string;
  readonly name: string;
  /** Shown on hover: the workspace path, or where a read-only document comes from. */
  readonly title: string;
  readonly dirty: boolean;
  readonly disk: DiskStatus;
  readonly saving: boolean;
  /** Text from elsewhere (an agent's worktree), shown for inspection only. */
  readonly readOnly: boolean;
}

export interface EditorSnapshot {
  readonly tabs: readonly TabInfo[];
  readonly active: string | null;
  /**
   * Increases whenever the active tab's state is replaced other than by the view
   * itself (opening, switching, reloading), telling the view to load it.
   */
  readonly revision: number;
  /** Increases when the view should scroll the active tab's selection into view. */
  readonly reveal: number;
}

interface Document {
  state: EditorState;
  /** The text as last read or saved; the tab is dirty when it differs. */
  saved: Text;
  version: FileVersion;
}

type EditorNative = Pick<WorkspaceApi, "readFile" | "writeFile" | "fileVersion">;

/**
 * Open files and their tabs. Holds each tab's full editor state (text, undo
 * history, selection), so switching tabs keeps them. React only sees the tab
 * metadata, which changes rarely: typing updates the state here without
 * re-rendering anything.
 */
export class EditorStore extends Store<EditorSnapshot> {
  readonly #native: EditorNative;
  readonly #documents = new Map<string, Document>();
  readonly #opening = new Map<string, Promise<void>>();

  constructor(native: EditorNative) {
    super({ tabs: [], active: null, revision: 0, reveal: 0 });
    this.#native = native;
  }

  stateOf(path: string): EditorState | undefined {
    return this.#documents.get(path)?.state;
  }

  isOpen(path: string): boolean {
    return this.#documents.has(path);
  }

  dirtyPaths(): string[] {
    return this.get()
      .tabs.filter((tab) => tab.dirty)
      .map((tab) => tab.path);
  }

  /** Opens the file in a tab, or switches to its tab. Read errors are thrown. */
  async open(path: string): Promise<void> {
    if (this.#documents.has(path)) {
      this.activate(path);
      return;
    }
    let opening = this.#opening.get(path);
    if (!opening) {
      opening = this.#read(path).finally(() => this.#opening.delete(path));
      this.#opening.set(path, opening);
    }
    await opening;
  }

  async #read(path: string): Promise<void> {
    const content = await this.#native.readFile(path);
    if (this.#documents.has(path)) {
      this.activate(path);
      return;
    }
    const state = createEditorState(content.text);
    this.#documents.set(path, { state, saved: state.doc, version: content.version });
    const tab: TabInfo = { path, name: basename(path), title: path, dirty: false, disk: "synced", saving: false, readOnly: false };
    this.update((s) => ({ ...s, tabs: [...s.tabs, tab], active: path, revision: s.revision + 1 }));
  }

  /**
   * Shows text that is not a file of the open workspace, read-only, in a tab of its
   * own: an agent's file or diff. `key` must start with `/`, so it never collides
   * with a workspace path; opening it again replaces the text.
   */
  openReadOnly(key: string, name: string, title: string, text: string): void {
    if (!key.startsWith("/")) throw new Error(`read-only documents need a key starting with "/", not ${key}`);
    const state = createEditorState(text, { readOnly: true });
    this.#documents.set(key, { state, saved: state.doc, version: "" });
    const tab: TabInfo = { path: key, name, title, dirty: false, disk: "synced", saving: false, readOnly: true };
    this.update((s) => ({
      ...s,
      tabs: s.tabs.some((t) => t.path === key) ? s.tabs.map((t) => (t.path === key ? tab : t)) : [...s.tabs, tab],
      active: key,
      revision: s.revision + 1,
    }));
  }

  activate(path: string): void {
    if (!this.#documents.has(path) || this.get().active === path) return;
    this.update((s) => ({ ...s, active: path, revision: s.revision + 1 }));
  }

  /**
   * Switches to an open file and selects `length` characters at a 1-based line
   * and a UTF-16 column (as search results give them), scrolled into view. A
   * position past the end of the file (it changed since) is clamped.
   */
  select(path: string, line: number, column: number, length: number): void {
    const document = this.#documents.get(path);
    if (!document) return;
    const { doc } = document.state;
    const target = doc.line(Math.min(Math.max(1, line), doc.lines));
    const from = Math.min(target.from + Math.max(0, column), target.to);
    const to = Math.min(from + Math.max(0, length), target.to);
    document.state = document.state.update({ selection: { anchor: from, head: to } }).state;
    this.update((s) => ({ ...s, active: path, revision: s.revision + 1, reveal: s.reveal + 1 }));
  }

  /** The view reports every change to the active document here. */
  applyViewState(path: string, state: EditorState): void {
    const document = this.#documents.get(path);
    if (!document || document.state === state) return;
    const textChanged = document.state.doc !== state.doc;
    document.state = state;
    if (textChanged) this.#refreshDirty(path);
  }

  /**
   * Saves the tab. Unless `overwrite` is set, the save goes ahead only if the file
   * on disk is still the version this tab last read or saved; otherwise it throws
   * a `conflict` error and writes nothing.
   */
  async save(path: string, { overwrite = false } = {}): Promise<void> {
    const document = this.#documents.get(path);
    if (!document || this.#tab(path)?.readOnly) return;
    const text = document.state.doc;
    const expected = overwrite ? null : document.version;
    this.#patch(path, { saving: true });
    try {
      const version = await this.#native.writeFile(path, textOf(document.state), expected);
      const current = this.#documents.get(path);
      if (current) {
        current.saved = text;
        current.version = version;
        this.#patch(path, { disk: "synced" });
        this.#refreshDirty(path);
      }
    } finally {
      this.#patch(path, { saving: false });
    }
  }

  /** Replaces the tab's text with the file on disk, discarding unsaved edits. */
  async reload(path: string): Promise<void> {
    if (this.#tab(path)?.readOnly) return;
    const content = await this.#native.readFile(path);
    const document = this.#documents.get(path);
    if (!document) return;
    // A transaction rather than a fresh state, so the reload itself can be undone.
    const state = document.state.update({
      changes: { from: 0, to: document.state.doc.length, insert: content.text },
    }).state;
    document.state = state;
    document.saved = state.doc;
    document.version = content.version;
    this.#patch(path, { disk: "synced" });
    this.#refreshDirty(path);
    if (this.get().active === path) this.update((s) => ({ ...s, revision: s.revision + 1 }));
  }

  /** Closes the tab, discarding unsaved edits. Callers confirm first. */
  close(path: string): void {
    if (!this.#documents.delete(path)) return;
    this.update((s) => {
      const index = s.tabs.findIndex((tab) => tab.path === path);
      const tabs = s.tabs.filter((tab) => tab.path !== path);
      if (s.active !== path) return { ...s, tabs };
      const next = tabs[Math.min(index, tabs.length - 1)]?.path ?? null;
      return { ...s, tabs, active: next, revision: s.revision + 1 };
    });
  }

  closeAll(): void {
    this.#documents.clear();
    this.update((s) => ({ ...s, tabs: [], active: null, revision: s.revision + 1 }));
  }

  /** A file or directory was renamed: tabs under it follow. */
  renamed(from: string, to: string): void {
    const moved = [...this.#documents.keys()].filter((path) => isWithin(path, from));
    if (moved.length === 0) return;
    for (const path of moved) {
      const document = this.#documents.get(path)!;
      this.#documents.delete(path);
      this.#documents.set(rebase(path, from, to), document);
    }
    this.update((s) => ({
      ...s,
      tabs: s.tabs.map((tab) => {
        if (!isWithin(tab.path, from)) return tab;
        const path = rebase(tab.path, from, to);
        return { ...tab, path, name: basename(path), title: path };
      }),
      active: s.active !== null && isWithin(s.active, from) ? rebase(s.active, from, to) : s.active,
      revision: s.revision + 1,
    }));
  }

  /**
   * Reconciles open tabs with changes on disk. An unmodified tab follows the
   * file; a tab with unsaved edits is only marked, never overwritten. The app's own
   * saves arrive here too and are recognised by their version.
   */
  async diskChanged(paths: readonly string[] | "all"): Promise<void> {
    const affected = [...this.#documents.keys()].filter(
      (open) => !open.startsWith("/") && (paths === "all" || paths.some((changed) => isWithin(open, changed))),
    );
    await Promise.all(affected.map((path) => this.#reconcile(path)));
  }

  async #reconcile(path: string): Promise<void> {
    let version: FileVersion | null;
    try {
      version = await this.#native.fileVersion(path);
    } catch {
      return; // Unreadable for now; the next change or save will surface it.
    }
    const document = this.#documents.get(path);
    const tab = this.#tab(path);
    if (!document || !tab || tab.saving) return;
    if (version === null) {
      this.#patch(path, { disk: "deleted" });
    } else if (version === document.version) {
      this.#patch(path, { disk: "synced" });
    } else if (!tab.dirty) {
      await this.reload(path);
    } else {
      this.#patch(path, { disk: "changed" });
    }
  }

  #refreshDirty(path: string): void {
    const document = this.#documents.get(path);
    if (!document) return;
    const { doc } = document.state;
    // The length check makes the common case (text differs) cheap for large files.
    const dirty = doc.length !== document.saved.length || !doc.eq(document.saved);
    this.#patch(path, { dirty });
  }

  #tab(path: string): TabInfo | undefined {
    return this.get().tabs.find((tab) => tab.path === path);
  }

  #patch(path: string, change: Partial<Pick<TabInfo, "dirty" | "disk" | "saving">>): void {
    const tab = this.#tab(path);
    if (!tab || Object.entries(change).every(([key, value]) => tab[key as keyof TabInfo] === value)) return;
    this.update((s) => ({ ...s, tabs: s.tabs.map((t) => (t.path === path ? { ...t, ...change } : t)) }));
  }
}
