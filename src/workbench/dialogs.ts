import { Store } from "../lib/store";

export interface DialogButton<T extends string> {
  readonly label: string;
  readonly value: T;
  readonly role?: "primary" | "destructive";
}

export interface DialogRequest<T extends string = string> {
  readonly title: string;
  readonly message: string;
  readonly buttons: readonly DialogButton<T>[];
  /** Chosen by Escape or clicking outside. */
  readonly cancel: T;
}

interface OpenDialog extends DialogRequest {
  readonly resolve: (value: string) => void;
}

/** Modal questions such as "save before closing?". One at a time. */
export class Dialogs extends Store<OpenDialog | null> {
  constructor() {
    super(null);
  }

  ask<T extends string>(request: DialogRequest<T>): Promise<T> {
    // A newer question replaces an unanswered one, which counts as cancelled.
    this.get()?.resolve(this.get()!.cancel);
    return new Promise<T>((resolve) => {
      this.set({ ...request, resolve: (value) => resolve(value as T) });
    });
  }

  answer(value: string): void {
    const open = this.get();
    if (!open) return;
    this.set(null);
    open.resolve(value);
  }
}
