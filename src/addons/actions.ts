/** What the Add-ons view asks of the workbench. */
export interface AddonActions {
  /** Looks again at what is installed (the login environment is read again). */
  refreshAddons(): void;
  /** Adds the add-on to the open space; installing first, once the user confirms. */
  addAddon(id: string): void;
  /** Takes it out of the open space; nothing is uninstalled. */
  removeAddon(id: string): void;
  /** Opens the welcome screen with `/share ` typed. */
  shareAddons(): void;
}
