import type { CatalogItem } from "../contracts/generated/CatalogItem";
import type { CatalogItemType } from "../contracts/generated/CatalogItemType";
import { Store } from "../lib/store";
import type { CatalogApi } from "../native";

export interface CatalogSnapshot {
  /** `null` until first loaded. */
  readonly items: readonly CatalogItem[] | null;
  readonly warnings: readonly string[];
  readonly loading: boolean;
  readonly error: string | null;
}

type CatalogNative = Pick<CatalogApi, "listCatalog">;

/**
 * The catalog as the native side assembles it from the systems that own each
 * item. Read-only: acting on an item goes through that system (the workbench).
 */
export class Catalog extends Store<CatalogSnapshot> {
  readonly #native: CatalogNative;
  #generation = 0;

  constructor(native: CatalogNative) {
    super({ items: null, warnings: [], loading: false, error: null });
    this.#native = native;
  }

  async load(): Promise<void> {
    const generation = ++this.#generation;
    this.update((s) => ({ ...s, loading: true }));
    try {
      const list = await this.#native.listCatalog();
      if (generation !== this.#generation) return;
      this.set({ items: list.items, warnings: list.warnings, loading: false, error: null });
    } catch (error) {
      if (generation !== this.#generation) return;
      this.update((s) => ({ ...s, loading: false, error: error instanceof Error ? error.message : String(error) }));
    }
  }

  find(id: string): CatalogItem | undefined {
    return this.get().items?.find((i) => i.id === id);
  }
}

export type CatalogCategory = "all" | CatalogItemType;
/** `usable`: installed or configured. */
export type StatusFilter = "any" | "usable" | "installed" | "configured";

export interface CatalogQuery {
  readonly text: string;
  readonly category: CatalogCategory;
  readonly status: StatusFilter;
}

export const TYPE_LABELS: Record<CatalogItemType, string> = {
  agent: "Agent",
  model: "Model",
  mcpServer: "MCP",
  skill: "Skill",
};

/** What search looks at: name, description, publisher or provider, tags and capabilities. */
export function searchText(item: CatalogItem): string {
  const provider = item.details.kind === "model" ? item.details.providerName : "";
  return [
    item.name,
    item.displayName,
    item.description,
    item.publisher ?? "",
    provider,
    TYPE_LABELS[item.type],
    ...item.tags,
    ...item.capabilities,
  ]
    .join("\n")
    .toLowerCase();
}

/** Every word of the text must match somewhere; category and status narrow further. Local and instant. */
export function filterCatalog(items: readonly CatalogItem[], query: CatalogQuery): CatalogItem[] {
  const words = query.text.toLowerCase().split(/\s+/).filter(Boolean);
  return items.filter((item) => {
    if (query.category !== "all" && item.type !== query.category) return false;
    switch (query.status) {
      case "usable":
        if (item.status !== "installed" && item.status !== "configured") return false;
        break;
      case "installed":
        if (item.status !== "installed") return false;
        break;
      case "configured":
        if (item.status !== "configured") return false;
        break;
      case "any":
        break;
    }
    if (words.length === 0) return true;
    const text = searchText(item);
    return words.every((w) => text.includes(w));
  });
}
