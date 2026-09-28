import type { SplitDirection } from "./panes";

/** What the terminal panel can ask for. The workbench implements it. */
export interface TerminalActions {
  newTerminal(): void;
  /** Splits the active tab's focused pane, starting a new shell beside or below it. */
  splitTerminal(direction: SplitDirection): void;
  /** Asks first if a program is running in the pane. */
  closeTerminalPane(key: number): void;
  /** Asks first if a program is running in any of the tab's panes. */
  closeTerminalTab(key: number): void;
}
