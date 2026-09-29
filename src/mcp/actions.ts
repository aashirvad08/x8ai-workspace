import type { McpServerInput } from "../contracts/generated/McpServerInput";

/** What the MCP view can ask for. The workbench implements it. */
export interface McpActions {
  refreshMcp(): void;
  /** Resolves to whether the server was added. */
  addMcpServer(server: McpServerInput): Promise<boolean>;
  /** Resolves to whether the server was changed. */
  updateMcpServer(id: string, server: McpServerInput): Promise<boolean>;
  setMcpServerEnabled(id: string, enabled: boolean): void;
  /** Removes the server and its secrets, after asking. */
  removeMcpServer(id: string): void;
  /** Saves a secret variable's value in the Keychain; resolves to whether it was saved. */
  saveMcpSecret(id: string, name: string, value: string): Promise<boolean>;
  /** Deletes a secret variable's saved value, after asking. */
  removeMcpSecret(id: string, name: string): void;
}
