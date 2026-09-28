import { Store } from "../lib/store";

export interface NotificationAction {
  readonly label: string;
  readonly run: () => void;
}

export interface Notification {
  readonly id: number;
  readonly tone: "info" | "error";
  readonly message: string;
  readonly actions: readonly NotificationAction[];
}

/** Information disappears after this long; errors stay until dismissed. */
const INFO_LIFETIME_MS = 4000;

/** Messages shown to the user. Errors are never swallowed: they end up here. */
export class Notifications extends Store<readonly Notification[]> {
  #nextId = 1;

  constructor() {
    super([]);
  }

  info(message: string): void {
    const id = this.#add({ tone: "info", message, actions: [] });
    setTimeout(() => this.dismiss(id), INFO_LIFETIME_MS);
  }

  error(message: string, actions: readonly NotificationAction[] = []): number {
    return this.#add({ tone: "error", message, actions });
  }

  dismiss(id: number): void {
    this.update((all) => all.filter((n) => n.id !== id));
  }

  #add(notification: Omit<Notification, "id">): number {
    const id = this.#nextId++;
    this.update((all) => [...all, { ...notification, id }]);
    return id;
  }
}

/** A readable message for anything thrown. */
export function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
