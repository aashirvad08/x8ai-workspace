export interface Shortcut {
  /** A letter, matched by the character typed, or "`", matched by key position. */
  readonly key: string;
  readonly meta?: boolean;
  readonly shift?: boolean;
  readonly ctrl?: boolean;
  readonly alt?: boolean;
}

export interface Command {
  readonly id: string;
  readonly title: string;
  readonly shortcut?: Shortcut;
  readonly run: () => void;
}

type KeyEvent = Pick<KeyboardEvent, "key" | "code" | "metaKey" | "shiftKey" | "ctrlKey" | "altKey">;

export function matches(shortcut: Shortcut, event: KeyEvent): boolean {
  const key = shortcut.key === "`" ? event.code === "Backquote" : event.key.toLowerCase() === shortcut.key;
  return (
    key &&
    event.metaKey === Boolean(shortcut.meta) &&
    event.shiftKey === Boolean(shortcut.shift) &&
    event.ctrlKey === Boolean(shortcut.ctrl) &&
    event.altKey === Boolean(shortcut.alt)
  );
}

/** macOS notation, modifiers in Apple's order: `⌃⌥⇧⌘P`. */
export function shortcutLabel(shortcut: Shortcut): string {
  return (
    (shortcut.ctrl ? "⌃" : "") +
    (shortcut.alt ? "⌥" : "") +
    (shortcut.shift ? "⇧" : "") +
    (shortcut.meta ? "⌘" : "") +
    shortcut.key.toUpperCase()
  );
}
