import type { AgentStatus } from "../contracts/generated/AgentStatus";
import type { ModelSelection } from "../contracts/generated/ModelSelection";
import type { ProviderStatus } from "../contracts/generated/ProviderStatus";
import { Store } from "../lib/store";
import type { ProviderApi } from "../native";

export interface ProvidersSnapshot {
  /** `null` until first loaded. */
  readonly providers: readonly ProviderStatus[] | null;
  readonly loading: boolean;
  readonly error: string | null;
}

type ProvidersNative = Pick<ProviderApi, "listProviders">;

/**
 * Model providers as the native side reports them: whether a key is saved (never
 * the key), local availability, and models. Changes go through the workbench,
 * which replaces a provider here with what the native side returns.
 */
export class Providers extends Store<ProvidersSnapshot> {
  readonly #native: ProvidersNative;
  #generation = 0;

  constructor(native: ProvidersNative) {
    super({ providers: null, loading: false, error: null });
    this.#native = native;
  }

  /** With `checkLocal`, the native side looks for Ollama on this machine first. */
  async load(checkLocal = false): Promise<void> {
    const generation = ++this.#generation;
    this.update((s) => ({ ...s, loading: true }));
    try {
      const list = await this.#native.listProviders(checkLocal);
      if (generation !== this.#generation) return;
      this.set({ providers: list.providers, loading: false, error: null });
    } catch (error) {
      if (generation !== this.#generation) return;
      this.update((s) => ({ ...s, loading: false, error: error instanceof Error ? error.message : String(error) }));
    }
  }

  /** Replaces one provider with its new status. */
  replace(status: ProviderStatus): void {
    this.update((s) => ({ ...s, providers: s.providers?.map((p) => (p.id === status.id ? status : p)) ?? null }));
  }

  find(id: string): ProviderStatus | undefined {
    return this.get().providers?.find((p) => p.id === id);
  }
}

/** A provider and model an agent can be launched with. */
export interface ModelChoice {
  readonly selection: ModelSelection;
  readonly provider: string;
  readonly model: string;
}

/**
 * What `agent` can be pointed at: models of providers its adapter supports that
 * have what they need (a saved key, or none needed), in provider order.
 */
export function modelChoices(agent: AgentStatus, providers: readonly ProviderStatus[]): ModelChoice[] {
  return providers.flatMap((provider) => {
    const supported = agent.providers.some((p) => p.provider === provider.id && p.supported);
    if (!supported || provider.credential === "missing") return [];
    return provider.models.map((model) => ({
      selection: { provider: provider.id, model: model.id },
      provider: provider.name,
      model: model.name,
    }));
  });
}
