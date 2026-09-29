/** What the agents view can ask for. The workbench implements it. */
export interface AgentActions {
  /** Checks trust and approval (asking the user as needed), then starts the agent in a terminal. */
  launchAgent(id: string): void;
  /** Forgets the agent's approval in the open workspace. */
  revokeAgent(id: string): void;
  /** Asks the native side to trust the open folder. */
  trustWorkspace(): void;
  /** Looks for installed agents again. */
  refreshAgents(): void;
}
