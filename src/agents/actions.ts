import type { ModelSelection } from "../contracts/generated/ModelSelection";

/** What the agents view can ask for. The workbench implements it. */
export interface AgentActions {
  /**
   * Checks trust and approval (asking the user as needed), creates a session and
   * starts the agent in a terminal: with its own model configuration (`model`
   * null), or pointed at `model`, and with the MCP servers and skills a new
   * session gets (the session-scoped ones in `mcp` and `skills` among them).
   */
  launchAgent(id: string, model: ModelSelection | null, mcp?: readonly string[], skills?: readonly string[]): void;
  /** Forgets the agent's approval in the open workspace. */
  revokeAgent(id: string): void;
  /** Asks the native side to trust the open folder. */
  trustWorkspace(): void;
  /** Looks for installed agents again. */
  refreshAgents(): void;
  /** Shows the session's terminal, starting the agent in it if it is not running. */
  openAgentTerminal(session: number): void;
  stopAgent(session: number): void;
  restartAgent(session: number): void;
  /** Loads the session's changes and opens its diff, read-only. */
  showAgentChanges(session: number): void;
  /** Opens one of the agent's files, read-only. */
  openAgentFile(session: number, path: string): void;
  /** Removes a stopped session and its worktree, asking first about anything that would be lost. */
  removeAgentSession(session: number): void;
}
