import type { AddonGroup } from "../contracts/generated/AddonGroup";
import type { AddonList } from "../contracts/generated/AddonList";
import type { AddonStatus } from "../contracts/generated/AddonStatus";
import type { SpaceInfo } from "../contracts/generated/SpaceInfo";
import { Store } from "../lib/store";
import type { AddonApi } from "../native";

/**
 * Add-ons in the open space (docs/decisions/0019-add-ons.md): tools its
 * terminals use. The native side owns the list and what each installs; this is
 * what the view shows.
 */
export interface AddonsSnapshot {
  /** `null` until first loaded. */
  readonly list: AddonList | null;
  readonly loading: boolean;
  readonly error: string | null;
  /** Add-ons being added or removed now, by id. */
  readonly busy: readonly string[];
}

type AddonsNative = Pick<AddonApi, "listAddons">;

/** The font family of the Nerd Font add-on: its "Mono" cut, whose icons fit one cell. */
export const NERD_FONT = '"JetBrainsMono Nerd Font Mono", "JetBrainsMono NFM", "SF Mono", Menlo, monospace';

export const GROUPS: readonly { group: AddonGroup; label: string }[] = [
  { group: "shell", label: "Shell" },
  { group: "editor", label: "Editor" },
  { group: "tools", label: "Tools" },
  { group: "look", label: "Look" },
];

export class Addons extends Store<AddonsSnapshot> {
  readonly #native: AddonsNative;
  #generation = 0;

  constructor(native: AddonsNative) {
    super({ list: null, loading: false, error: null, busy: [] });
    this.#native = native;
  }

  async load(refresh = false): Promise<void> {
    const generation = ++this.#generation;
    this.update((s) => ({ ...s, loading: true }));
    try {
      const list = await this.#native.listAddons(refresh);
      if (generation !== this.#generation) return;
      this.update((s) => ({ ...s, list, loading: false, error: null }));
    } catch (error) {
      if (generation !== this.#generation) return;
      this.update((s) => ({ ...s, loading: false, error: error instanceof Error ? error.message : String(error) }));
    }
  }

  /** A list the native side returned after a change. Supersedes a load in flight. */
  setList(list: AddonList): void {
    ++this.#generation;
    this.update((s) => ({ ...s, list, loading: false, error: null }));
  }

  setBusy(id: string, busy: boolean): void {
    this.update((s) => ({ ...s, busy: busy ? [...s.busy.filter((b) => b !== id), id] : s.busy.filter((b) => b !== id) }));
  }

  find(id: string): AddonStatus | undefined {
    return this.get().list?.addons.find((a) => a.id === id);
  }

  /** The open space's terminal font, if its font add-on is on. */
  font(): string | null {
    return this.find("nerd-font")?.active ? NERD_FONT : null;
  }

  /** The add-ons added to the open space, in list order. */
  added(): AddonStatus[] {
    return this.get().list?.addons.filter((a) => a.added) ?? [];
  }
}

/** Why an added add-on is not on in new terminals, or `null` if it is. */
export function offReason(addon: AddonStatus, list: AddonList): string | null {
  if (!addon.added || addon.active) return null;
  if (!addon.installed) return "Not installed on this Mac any more. Add it again to install it.";
  if (addon.needsTrust) return "Turns on once you trust this folder: it runs Git here.";
  if (!list.shellSupported) return `Needs zsh; your shell is ${list.shell}.`;
  return "Off.";
}

/**
 * The spaces `/share <arg>` means, other than the open one: the one with that
 * id; else those with that folder or name (ignoring case); else those whose
 * name starts with it.
 */
export function matchSpaces(arg: string, spaces: readonly SpaceInfo[], open: string | null): SpaceInfo[] {
  const wanted = arg.trim();
  if (wanted === "") return [];
  const others = spaces.filter((s) => s.id !== open);
  const byId = others.filter((s) => s.id === wanted.toLowerCase());
  if (byId.length > 0) return byId;
  const lower = wanted.toLowerCase();
  const exact = others.filter((s) => s.root === wanted || s.name.toLowerCase() === lower);
  if (exact.length > 0) return exact;
  return others.filter((s) => s.name.toLowerCase().startsWith(lower));
}

/** How `/share` names a space: by name when no other space has it, else by id. */
export function shareName(space: SpaceInfo, spaces: readonly SpaceInfo[]): string {
  return spaces.filter((s) => s.name.toLowerCase() === space.name.toLowerCase()).length > 1 ? space.id : space.name;
}
