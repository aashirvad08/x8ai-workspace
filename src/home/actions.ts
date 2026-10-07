/** What the welcome screen asks of the workbench. */
export interface HomeActions {
  /** Runs `/cd`, `/new`, `/share`, `/home`, `/name`, `/get` or `/give`; anything else is explained on the screen. */
  runHomeCommand(text: string): Promise<void>;
  /** `/share` with the add-ons chosen: gives them to the space `to`. Resolves to whether it did. */
  shareSpace(to: string, addons: readonly string[]): Promise<boolean>;
  /** Back to the workspace as it is. */
  leaveHome(): void;
}
