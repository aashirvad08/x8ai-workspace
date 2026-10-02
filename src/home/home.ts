import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import { Store } from "../lib/store";
import type { AppApi } from "../native";

/**
 * The welcome screen, the head of the app: where it starts, and where the user
 * types `/cd` to open a space (a folder) and `/home` for the workspace with no
 * folder. One space is open at a time; the head shows over it and leaves it
 * running.
 */
export interface HomeState {
  readonly visible: boolean;
  /** The name chosen with `/name`; `null` for the account's own. */
  readonly chosenName: string | null;
  /** The account's full name, as the native side reports it. */
  readonly accountName: string | null;
  readonly message: HomeMessage | null;
}

export interface HomeMessage {
  readonly text: string;
  readonly tone: "info" | "error";
}

const NAME_KEY = "x8ai.name";
/** Longest name the welcome greets. */
export const MAX_NAME_LENGTH = 40;

export class Home extends Store<HomeState> {
  readonly #native: Pick<AppApi, "getAppInfo">;

  constructor(native: Pick<AppApi, "getAppInfo">) {
    super({ visible: true, chosenName: loadName(), accountName: null, message: null });
    this.#native = native;
  }

  async load(): Promise<void> {
    try {
      const info = await this.#native.getAppInfo();
      this.update((s) => ({ ...s, accountName: info.userName }));
    } catch {
      // Only the greeting's name is missing; the connection error is shown elsewhere.
    }
  }

  /** Who the welcome greets. */
  name(): string | null {
    const { chosenName, accountName } = this.get();
    return chosenName ?? accountName;
  }

  show(): void {
    this.update((s) => ({ ...s, visible: true, message: null }));
  }

  hide(): void {
    this.update((s) => ({ ...s, visible: false, message: null }));
  }

  say(text: string, tone: HomeMessage["tone"] = "info"): void {
    this.update((s) => ({ ...s, message: { text, tone } }));
  }

  /** Remembers the name on this machine; `null` goes back to the account's. */
  setName(name: string | null): void {
    const chosen = name?.trim().slice(0, MAX_NAME_LENGTH) || null;
    try {
      if (chosen) localStorage.setItem(NAME_KEY, chosen);
      else localStorage.removeItem(NAME_KEY);
    } catch {
      // Storage unavailable: the name lasts until the app quits.
    }
    this.update((s) => ({ ...s, chosenName: chosen }));
  }
}

function loadName(): string | null {
  try {
    return localStorage.getItem(NAME_KEY)?.slice(0, MAX_NAME_LENGTH) || null;
  } catch {
    return null;
  }
}

// Commands

export type HomeCommandName = "/cd" | "/home" | "/name" | "/get" | "/give";

export const HOME_COMMANDS: readonly { name: HomeCommandName; usage: string; description: string }[] = [
  { name: "/cd", usage: "/cd <folder>", description: "open a folder as your space" },
  { name: "/home", usage: "/home", description: "the workspace with no folder open" },
  { name: "/name", usage: "/name <your name>", description: "how the welcome greets you" },
  { name: "/get", usage: "/get [agent]", description: "give an agent what other sessions did" },
  { name: "/give", usage: "/give [agent]", description: "pass an agent's work to another session" },
];

export type ParsedCommand =
  | { readonly kind: "empty" }
  | { readonly kind: "command"; readonly name: HomeCommandName; readonly arg: string }
  | { readonly kind: "unknown"; readonly word: string };

export function parseCommand(text: string): ParsedCommand {
  const trimmed = text.trim();
  if (trimmed === "") return { kind: "empty" };
  const space = trimmed.search(/\s/);
  const word = space === -1 ? trimmed : trimmed.slice(0, space);
  const arg = space === -1 ? "" : trimmed.slice(space).trim();
  const command = HOME_COMMANDS.find((c) => c.name === word);
  return command ? { kind: "command", name: command.name, arg } : { kind: "unknown", word };
}

export interface Suggestion {
  readonly label: string;
  readonly detail: string;
  /** What the input becomes when the suggestion is taken. */
  readonly completion: string;
}

/** What fits what is typed so far: commands, then for `/cd`, recent spaces. */
export function suggestionsFor(text: string, recent: readonly RecentWorkspace[]): Suggestion[] {
  if (!text.startsWith("/")) return [];
  const space = text.search(/\s/);
  if (space === -1) {
    return HOME_COMMANDS.filter((c) => c.name.startsWith(text)).map((c) => ({
      label: c.usage,
      detail: c.description,
      completion: c.name === "/home" || c.name === "/get" || c.name === "/give" ? c.name : `${c.name} `,
    }));
  }
  if (text.slice(0, space) !== "/cd") return [];
  const query = text.slice(space).trim().toLowerCase();
  return recent
    .filter((r) => r.available && (query === "" || r.root.toLowerCase().includes(query.replace(/^~\//, "/"))))
    .slice(0, 8)
    .map((r) => ({ label: r.name, detail: r.root, completion: `/cd ${r.root}` }));
}

/**
 * The recent spaces `/cd <arg>` means: the exact folder, or every one whose path
 * ends with `arg` (`~/` taken as a prefix of the home folder), or else every one
 * whose name starts with it. `/cd gymRL` and `/cd gym` match `/Users/me/gymRL`.
 */
export function matchRecent(arg: string, recent: readonly RecentWorkspace[]): RecentWorkspace[] {
  const wanted = arg.trim().replace(/\/+$/, "");
  if (wanted === "" || wanted === "~") return [];
  const available = recent.filter((r) => r.available);
  if (wanted.startsWith("/")) return available.filter((r) => r.root === wanted);
  const relative = wanted.replace(/^~\//, "");
  const byPath = available.filter((r) => r.root.endsWith(`/${relative}`));
  if (byPath.length > 0 || relative.includes("/")) return byPath;
  const prefix = relative.toLowerCase();
  return available.filter((r) => r.name.toLowerCase().startsWith(prefix));
}
