import type { AgentStatus } from "../contracts/generated/AgentStatus";
import type { McpServerStatus } from "../contracts/generated/McpServerStatus";
import { Store } from "../lib/store";
import type { McpApi } from "../native";

export interface McpSnapshot {
  /** `null` until first loaded. */
  readonly servers: readonly McpServerStatus[] | null;
  readonly loading: boolean;
  readonly error: string | null;
}

type McpNative = Pick<McpApi, "listMcpServers">;

/**
 * The MCP servers the user configured, as the native side reports them: whether
 * their secrets are saved (never the secrets), whether they are ready, and which
 * agents can use them. Changes go through the workbench.
 */
export class McpServers extends Store<McpSnapshot> {
  readonly #native: McpNative;
  #generation = 0;

  constructor(native: McpNative) {
    super({ servers: null, loading: false, error: null });
    this.#native = native;
  }

  async load(): Promise<void> {
    const generation = ++this.#generation;
    this.update((s) => ({ ...s, loading: true }));
    try {
      const list = await this.#native.listMcpServers();
      if (generation !== this.#generation) return;
      this.set({ servers: list.servers, loading: false, error: null });
    } catch (error) {
      if (generation !== this.#generation) return;
      this.update((s) => ({ ...s, loading: false, error: error instanceof Error ? error.message : String(error) }));
    }
  }

  /** Adds or replaces one server with its new status. */
  replace(status: McpServerStatus): void {
    this.update((s) => {
      const servers = s.servers ?? [];
      const known = servers.some((p) => p.server.id === status.server.id);
      return {
        ...s,
        servers: known ? servers.map((p) => (p.server.id === status.server.id ? status : p)) : [...servers, status],
      };
    });
  }

  forget(id: string): void {
    this.update((s) => ({ ...s, servers: s.servers?.filter((p) => p.server.id !== id) ?? null }));
  }

  find(id: string): McpServerStatus | undefined {
    return this.get().servers?.find((p) => p.server.id === id);
  }
}

/** What a new session of `agent` in the folder `root` would get. */
export interface McpChoices {
  /** Attached to every new session here: enabled global and workspace servers. */
  readonly always: readonly McpServerStatus[];
  /** Enabled session servers, to choose from at launch. */
  readonly optional: readonly McpServerStatus[];
}

/** The servers `agent` can use in `root`, as the native side would attach them. */
export function mcpChoices(agent: AgentStatus, servers: readonly McpServerStatus[], root: string | null): McpChoices {
  if (!agent.mcp.supported) return { always: [], optional: [] };
  const usable = servers.filter(
    (s) => s.server.enabled && s.agents.some((a) => a.agent === agent.id && a.supported),
  );
  return {
    always: usable.filter(
      (s) => s.server.scope.kind === "global" || (s.server.scope.kind === "workspace" && s.server.scope.root === root),
    ),
    optional: usable.filter((s) => s.server.scope.kind === "session"),
  };
}
