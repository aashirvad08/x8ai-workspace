import type { AgentChanges } from "../contracts/generated/AgentChanges";
import type { AgentSessionInfo } from "../contracts/generated/AgentSessionInfo";
import type { AgentStatus } from "../contracts/generated/AgentStatus";
import type { WorkspaceIsolation } from "../contracts/generated/WorkspaceIsolation";
import { Store } from "../lib/store";
import type { AgentApi } from "../native";

export interface AgentsSnapshot {
  /** `null` until first loaded. */
  readonly agents: readonly AgentStatus[] | null;
  readonly loading: boolean;
  /** Why the user's login environment could not be read, if it could not. */
  readonly environmentProblem: string | null;
  /** How agents run in the open workspace: worktrees of their own, or not isolated. */
  readonly isolation: WorkspaceIsolation | null;
  /** The open workspace's agent sessions, oldest first. */
  readonly sessions: readonly AgentSessionInfo[];
  /** Changes loaded for review, by session. */
  readonly changes: ReadonlyMap<number, AgentChanges>;
  readonly error: string | null;
}

type AgentsNative = Pick<AgentApi, "listAgents" | "agentSessions">;

/**
 * The built-in agents and the open workspace's agent sessions, as the native side
 * reports them. What a running agent shows is in its terminal pane.
 */
export class Agents extends Store<AgentsSnapshot> {
  readonly #native: AgentsNative;
  #generation = 0;
  #sessionsGeneration = 0;

  constructor(native: AgentsNative) {
    super({ agents: null, loading: false, environmentProblem: null, isolation: null, sessions: [], changes: new Map(), error: null });
    this.#native = native;
  }

  /** Asks the native side again; with `refresh`, it re-reads the user's `PATH` first. */
  async load(refresh = false): Promise<void> {
    const generation = ++this.#generation;
    this.update((s) => ({ ...s, loading: true }));
    try {
      const list = await this.#native.listAgents(refresh);
      if (generation !== this.#generation) return;
      this.update((s) => ({
        ...s,
        agents: list.agents,
        loading: false,
        environmentProblem: list.environmentProblem,
        isolation: list.isolation,
        error: null,
      }));
    } catch (error) {
      if (generation !== this.#generation) return;
      this.update((s) => ({ ...s, loading: false, error: messageOf(error) }));
    }
    await this.loadSessions();
  }

  async loadSessions(): Promise<void> {
    const generation = ++this.#sessionsGeneration;
    try {
      const sessions = await this.#native.agentSessions();
      if (generation !== this.#sessionsGeneration) return;
      this.update((s) => {
        // Changes of sessions that are gone are no longer worth keeping.
        const changes = new Map([...s.changes].filter(([id]) => sessions.some((session) => session.id === id)));
        return { ...s, sessions, changes };
      });
    } catch (error) {
      if (generation !== this.#sessionsGeneration) return;
      this.update((s) => ({ ...s, error: messageOf(error) }));
    }
  }

  setChanges(session: number, changes: AgentChanges): void {
    this.update((s) => ({ ...s, changes: new Map(s.changes).set(session, changes) }));
  }

  find(id: string): AgentStatus | undefined {
    return this.get().agents?.find((agent) => agent.id === id);
  }

  session(id: number): AgentSessionInfo | undefined {
    return this.get().sessions.find((session) => session.id === id);
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
