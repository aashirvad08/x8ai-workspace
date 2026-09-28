/** What the editor area can ask for. The workbench implements it. */
export interface EditorActions {
  save(path: string): void;
  /** Asks first if the tab has unsaved changes. */
  close(path: string): void;
  /** Saves over whatever is on disk now. */
  overwrite(path: string): void;
  /** Discards unsaved edits and reloads from disk. */
  revert(path: string): void;
}
