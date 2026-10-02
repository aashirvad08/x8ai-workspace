import type { ModelSelection } from "../contracts/generated/ModelSelection";
import type { ShareMode, ShareParts } from "./share";

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
  /**
   * Opens the context composer: `get`, for `session` to receive context from the
   * other sessions; `give`, for `session` to give its context to another.
   * Without a session, the focused agent's.
   */
  shareContext(mode: ShareMode, session?: number | null): void;
}

/** What the context composer can ask for. The workbench implements it. */
export interface ContextActions {
  setShareSources(sources: readonly number[]): void;
  setShareTarget(target: number | null): void;
  setShareParts(parts: ShareParts): void;
  setShareNote(note: string): void;
  /** The user's own edit of the text; choosing again composes it anew. */
  editShareText(text: string): void;
  /** Pastes the text into the receiving agent's input, starting it first if the user agrees. Never presses Enter. */
  sendShareContext(): void;
  closeShareContext(): void;
}
