import type { AgentStatus } from "../contracts/generated/AgentStatus";
import { Store } from "../lib/store";
import type { AgentApi } from "../native";

export interface AgentsSnapshot {
  /** `null` until first loaded. */
  readonly agents: readonly AgentStatus[] | null;
  readonly loading: boolean;
  /** Why the user's login environment could not be read, if it could not. */
  readonly environmentProblem: string | null;
  readonly error: string | null;
}

/**
 * The built-in agents as the native side sees them: installed or not, and approved
 * for the open workspace or not. What is running is in the terminal panes.
 */
export class Agents extends Store<AgentsSnapshot> {
  readonly #native: Pick<AgentApi, "listAgents">;
  #generation = 0;

  constructor(native: Pick<AgentApi, "listAgents">) {
    super({ agents: null, loading: false, environmentProblem: null, error: null });
    this.#native = native;
  }

  /** Asks the native side again; with `refresh`, it re-reads the user's `PATH` first. */
  async load(refresh = false): Promise<void> {
    const generation = ++this.#generation;
    this.update((s) => ({ ...s, loading: true }));
    try {
      const list = await this.#native.listAgents(refresh);
      if (generation !== this.#generation) return;
      this.set({ agents: list.agents, loading: false, environmentProblem: list.environmentProblem, error: null });
    } catch (error) {
      if (generation !== this.#generation) return;
      const message = error instanceof Error ? error.message : String(error);
      this.update((s) => ({ ...s, loading: false, error: message }));
    }
  }

  find(id: string): AgentStatus | undefined {
    return this.get().agents?.find((agent) => agent.id === id);
  }
}
