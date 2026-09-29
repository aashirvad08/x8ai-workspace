/** What the models view can ask for. The workbench implements it. */
export interface ModelActions {
  /** Looks for local providers (Ollama) again and reloads every provider. */
  refreshProviders(): void;
  /** Saves the key in the Keychain; resolves to whether it was saved. */
  saveProviderKey(provider: string, key: string): Promise<boolean>;
  /** Removes the saved key, after asking. */
  removeProviderKey(provider: string): void;
  /** Resolves to whether the model id was added. */
  addProviderModel(provider: string, model: string): Promise<boolean>;
  removeProviderModel(provider: string, model: string): void;
}
