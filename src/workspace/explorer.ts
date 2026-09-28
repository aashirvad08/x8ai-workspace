import type { DirEntry } from "../contracts/generated/DirEntry";
import { dirname, isWithin, rebase } from "../lib/paths";
import { Store } from "../lib/store";
import type { WorkspaceApi } from "../native";

/** One directory's listing. Both fields `null` while it loads. */
export interface Listing {
  readonly entries: readonly DirEntry[] | null;
  readonly error: string | null;
}

/** An inline name field in the tree: creating an entry in `parent`, or renaming `path`. */
export type Editing =
  | { readonly kind: "newFile" | "newFolder"; readonly parent: string }
  | { readonly kind: "rename"; readonly path: string };

export interface ExplorerSnapshot {
  /** Loaded directories, by workspace path (`""` is the root). Nothing else is read. */
  readonly listings: ReadonlyMap<string, Listing>;
  readonly expanded: ReadonlySet<string>;
  readonly selected: string | null;
  readonly editing: Editing | null;
}

/** A row of the visible tree. */
export type Row =
  | { readonly type: "entry"; readonly entry: DirEntry; readonly depth: number; readonly expanded: boolean }
  | { readonly type: "status"; readonly dir: string; readonly depth: number; readonly text: string; readonly error: boolean }
  | { readonly type: "new"; readonly editing: Editing; readonly depth: number };

const EMPTY: ExplorerSnapshot = { listings: new Map(), expanded: new Set(), selected: null, editing: null };

/**
 * The file tree. Directories are listed one level at a time, and only once
 * expanded, so a large repository is never read as a whole.
 */
export class Explorer extends Store<ExplorerSnapshot> {
  readonly #native: Pick<WorkspaceApi, "listDir">;

  constructor(native: Pick<WorkspaceApi, "listDir">) {
    super(EMPTY);
    this.#native = native;
  }

  /** Starts over for a newly opened workspace (or none). */
  reset(open: boolean): void {
    this.set(open ? { ...EMPTY, expanded: new Set([""]) } : EMPTY);
    if (open) void this.load("");
  }

  /** Lists `dir` again. A directory that has disappeared is forgotten. */
  async load(dir: string): Promise<void> {
    if (!this.get().listings.has(dir)) this.#setListing(dir, { entries: null, error: null });
    try {
      const entries = await this.#native.listDir(dir);
      if (this.get().listings.has(dir)) this.#setListing(dir, { entries, error: null });
    } catch (error) {
      const { code, message } = error as { code?: string; message?: string };
      if (code === "notFound" && dir !== "") {
        this.removed(dir);
      } else if (this.get().listings.has(dir)) {
        this.#setListing(dir, { entries: [], error: message ?? String(error) });
      }
    }
  }

  toggle(dir: string): void {
    if (this.get().expanded.has(dir)) {
      this.update((s) => ({ ...s, expanded: without(s.expanded, dir) }));
    } else {
      this.expand(dir);
    }
  }

  expand(dir: string): void {
    this.update((s) => ({ ...s, expanded: new Set(s.expanded).add(dir) }));
    if (!this.get().listings.has(dir)) void this.load(dir);
  }

  collapseAll(): void {
    this.update((s) => ({ ...s, expanded: new Set([""]) }));
  }

  select(path: string | null): void {
    this.update((s) => (s.selected === path ? s : { ...s, selected: path }));
  }

  startEditing(editing: Editing): void {
    if (editing.kind !== "rename") this.expand(editing.parent);
    this.update((s) => ({ ...s, editing }));
  }

  stopEditing(): void {
    this.update((s) => (s.editing === null ? s : { ...s, editing: null }));
  }

  /** Reloads the loaded directories that contain, or are, a changed path. */
  async diskChanged(paths: readonly string[] | "all"): Promise<void> {
    const loaded = [...this.get().listings.keys()];
    const stale =
      paths === "all"
        ? loaded
        : loaded.filter((dir) => paths.some((changed) => dirname(changed) === dir || changed === dir));
    await Promise.all(stale.map((dir) => this.load(dir)));
  }

  /** Keeps expansion and selection when a file or directory is renamed. */
  renamed(from: string, to: string): void {
    this.update((s) => ({
      ...s,
      listings: mapKeys(s.listings, (dir) => (isWithin(dir, from) ? null : dir)),
      expanded: new Set([...s.expanded].map((dir) => (isWithin(dir, from) && from !== "" ? rebase(dir, from, to) : dir))),
      selected: s.selected !== null && isWithin(s.selected, from) ? rebase(s.selected, from, to) : s.selected,
    }));
    // Moved directories that were open are listed again under their new path.
    const { expanded, listings } = this.get();
    for (const dir of expanded) if (!listings.has(dir)) void this.load(dir);
  }

  /** Forgets a removed entry and everything beneath it. */
  removed(path: string): void {
    this.update((s) => ({
      ...s,
      listings: mapKeys(s.listings, (dir) => (isWithin(dir, path) ? null : dir)),
      expanded: new Set([...s.expanded].filter((dir) => !isWithin(dir, path))),
      selected: s.selected !== null && isWithin(s.selected, path) ? null : s.selected,
    }));
  }

  #setListing(dir: string, listing: Listing): void {
    this.update((s) => ({ ...s, listings: new Map(s.listings).set(dir, listing) }));
  }
}

/** The visible tree, flattened in display order, including inline name fields. */
export function visibleRows(snapshot: ExplorerSnapshot): Row[] {
  const rows: Row[] = [];
  const { editing } = snapshot;
  const walk = (dir: string, depth: number) => {
    if (editing && editing.kind !== "rename" && editing.parent === dir) rows.push({ type: "new", editing, depth });
    const listing = snapshot.listings.get(dir);
    if (!listing || listing.entries === null) {
      rows.push({ type: "status", dir, depth, text: "Loading…", error: false });
      return;
    }
    if (listing.error !== null) {
      rows.push({ type: "status", dir, depth, text: listing.error, error: true });
      return;
    }
    for (const entry of listing.entries) {
      const expanded = entry.kind === "directory" && snapshot.expanded.has(entry.path);
      rows.push({ type: "entry", entry, depth, expanded });
      if (expanded) walk(entry.path, depth + 1);
    }
  };
  if (snapshot.expanded.has("")) walk("", 0);
  return rows;
}

function without<T>(set: ReadonlySet<T>, value: T): Set<T> {
  const next = new Set(set);
  next.delete(value);
  return next;
}

function mapKeys<V>(map: ReadonlyMap<string, V>, key: (k: string) => string | null): Map<string, V> {
  const next = new Map<string, V>();
  for (const [k, v] of map) {
    const mapped = key(k);
    if (mapped !== null) next.set(mapped, v);
  }
  return next;
}
