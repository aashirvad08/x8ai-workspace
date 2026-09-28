import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import { Store } from "../lib/store";
import type { Command } from "./commands";

/** What the picker (⌘P, ⇧⌘P, ⌃R) is choosing from. */
export type PickerState =
  | { readonly kind: "files"; readonly files: readonly string[] | null; readonly truncated: boolean }
  | { readonly kind: "commands"; readonly commands: readonly Command[] }
  | { readonly kind: "workspaces"; readonly workspaces: readonly RecentWorkspace[] };

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

  showWorkspaces(workspaces: readonly RecentWorkspace[]): void {
    this.set({ kind: "workspaces", workspaces });
  }

  close(): void {
    this.set(null);
  }
}
