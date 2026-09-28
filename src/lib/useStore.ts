import { useSyncExternalStore } from "react";

import type { Store } from "./store";

/** Re-renders the component whenever the store's snapshot changes. */
export function useStore<T>(store: Store<T>): T {
  return useSyncExternalStore(store.subscribe, store.get);
}
