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
  /**
   * Where the shortcut applies: only while a terminal has keyboard focus, or
   * only while one does not. Everywhere if unset. The command palette runs any
   * command regardless.
   */
  readonly when?: "terminalFocused" | "terminalNotFocused";
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

/** Whether the command's shortcut applies, given where keyboard focus is. */
export function applies(command: Command, terminalFocused: boolean): boolean {
  if (command.when === "terminalFocused") return terminalFocused;
  if (command.when === "terminalNotFocused") return !terminalFocused;
  return true;
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
