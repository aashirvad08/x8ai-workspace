import { Store } from "../lib/store";
import type { Command } from "./commands";

/** What the picker (Cmd+P, Cmd+Shift+P) is choosing from. */
export type PickerState =
  | { readonly kind: "files"; readonly files: readonly string[] | null; readonly truncated: boolean }
  | { readonly kind: "commands"; readonly commands: readonly Command[] };

export class Picker extends Store<PickerState | null> {
  constructor() {
    super(null);
  }

  showFiles(files: readonly string[] | null, truncated = false): void {
    this.set({ kind: "files", files, truncated });
  }

  showCommands(commands: readonly Command[]): void {
    this.set({ kind: "commands", commands });
  }

  close(): void {
    this.set(null);
  }
}
