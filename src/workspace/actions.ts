import type { DirEntry } from "../contracts/generated/DirEntry";

/** What the file explorer can ask for. The workbench implements it. */
export interface ExplorerActions {
  openFolder(): void;
  openFile(path: string): void;
  /** Resolves to whether the entry was created. */
  create(parent: string, name: string, kind: "file" | "folder"): Promise<boolean>;
  /** Resolves to whether the entry was renamed. */
  rename(path: string, name: string): Promise<boolean>;
  /** Asks for confirmation, then moves the entry to the Trash. */
  remove(entry: DirEntry): void;
  /** Shows an inline name field in the selected folder (or next to the selected file). */
  startCreating(kind: "file" | "folder"): void;
}
