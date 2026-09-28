import type { DirEntry } from "../contracts/generated/DirEntry";
import type { SearchMatch } from "../contracts/generated/SearchMatch";

/** What the file explorer can ask for. The workbench implements it. */
export interface ExplorerActions {
  openFolder(): void;
  /** Reopens a folder from the recent list. */
  openRecent(root: string): void;
  /** Removes a folder from the recent list. */
  forgetRecent(root: string): void;
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

/** What the search view can ask for. The workbench implements it. */
export interface SearchActions {
  /** Opens the file with the match selected. */
  openMatch(path: string, match: SearchMatch): void;
}
