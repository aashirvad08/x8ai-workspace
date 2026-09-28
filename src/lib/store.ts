/**
 * A value that changes over time and can be observed. The state stores
 * (workspace, editor, terminals, notifications) build on it, so their logic stays
 * plain TypeScript, testable without React. React reads them with `useStore`.
 *
 * Snapshots are immutable: `set` replaces the value, and subscribers are notified
 * only when it actually changes.
 */
export class Store<T> {
  #value: T;
  readonly #listeners = new Set<() => void>();

  constructor(initial: T) {
    this.#value = initial;
  }

  readonly get = (): T => this.#value;

  readonly subscribe = (listener: () => void): (() => void) => {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  };

  protected set(value: T): void {
    if (Object.is(value, this.#value)) return;
    this.#value = value;
    for (const listener of [...this.#listeners]) listener();
  }

  protected update(change: (value: T) => T): void {
    this.set(change(this.#value));
  }
}

/** A store whose value anyone may set. */
export class Value<T> extends Store<T> {
  override set(value: T): void {
    super.set(value);
  }
}
