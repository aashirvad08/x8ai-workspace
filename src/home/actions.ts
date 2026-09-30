/** What the welcome screen asks of the workbench. */
export interface HomeActions {
  /** Runs `/cd`, `/home` or `/name`; anything else is explained on the screen. */
  runHomeCommand(text: string): Promise<void>;
  /** Back to the workspace as it is. */
  leaveHome(): void;
}
