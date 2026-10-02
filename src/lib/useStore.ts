import { useMemo, useSyncExternalStore } from "react";

import type { Store } from "./store";

/** Re-renders the component whenever the store's snapshot changes. */
export function useStore<T>(store: Store<T>): T {
  return useSyncExternalStore(store.subscribe, store.get);
}

/**
 * Re-renders the component only when the part of the store it uses changes:
 * `select` picks that part, and `equal` says whether two picks are the same.
 * For a component that needs little of a store that changes often, such as the
 * terminals (every focus change, tab switch and split drag).
 */
export function useSelected<T, S>(store: Store<T>, select: (value: T) => S, equal: (a: S, b: S) => boolean = Object.is): S {
  const picker = useMemo(() => selection(store, select, equal), [store]);
  // The latest functions, so they may be written inline.
  picker.update(select, equal);
  return useSyncExternalStore(store.subscribe, picker.get);
}

/**
 * The selected part of `store`, as a snapshot function for
 * `useSyncExternalStore`: the same value (by identity) for as long as `equal`
 * says the selection has not changed, so React skips the render.
 */
export function selection<T, S>(
  store: Store<T>,
  select: (value: T) => S,
  equal: (a: S, b: S) => boolean,
): { get: () => S; update: (select: (value: T) => S, equal: (a: S, b: S) => boolean) => void } {
  let current = { select, equal };
  let cached: { snapshot: T; selected: S } | undefined;
  return {
    update: (nextSelect, nextEqual) => {
      current = { select: nextSelect, equal: nextEqual };
    },
    get: () => {
      const snapshot = store.get();
      if (cached && cached.snapshot === snapshot) return cached.selected;
      const selected = current.select(snapshot);
      if (cached && current.equal(cached.selected, selected)) {
        cached = { snapshot, selected: cached.selected };
        return cached.selected;
      }
      cached = { snapshot, selected };
      return selected;
    },
  };
}

/** Whether two arrays hold the same items, by identity, in the same order. */
export function sameItems<T>(a: readonly T[], b: readonly T[]): boolean {
  return a.length === b.length && a.every((item, i) => item === b[i]);
}
