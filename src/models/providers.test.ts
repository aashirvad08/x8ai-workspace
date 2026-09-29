import { describe, expect, it } from "vitest";

import type { AgentStatus } from "../contracts/generated/AgentStatus";
import type { ProviderStatus } from "../contracts/generated/ProviderStatus";
import { modelChoices, Providers } from "./providers";

const agent: AgentStatus = {
  id: "claude-code",
  name: "Claude Code",
  description: "",
  availability: { state: "installed", executable: "/usr/local/bin/claude" },
  approved: false,
  providers: [
    { provider: "anthropic", supported: true, reason: null },
    { provider: "openai", supported: false, reason: "Claude Code speaks only the Anthropic Messages API" },
    { provider: "ollama", supported: true, reason: null },
    { provider: "openrouter", supported: true, reason: null },
  ],
  mcp: { supported: true, reason: null },
};

function provider(id: string, credential: ProviderStatus["credential"], models: string[]): ProviderStatus {
  return {
    id,
    name: id,
    description: "",
    hosting: credential === "notNeeded" ? "local" : "hosted",
    credential,
    local: null,
    models: models.map((m) => ({ id: m, provider: id, name: m, source: "builtIn", contextWindow: null })),
  };
}

describe("model choices", () => {
  it("offers models of supported providers that can authenticate, in order", () => {
    const choices = modelChoices(agent, [
      provider("anthropic", "inKeychain", ["claude-sonnet-5", "claude-opus-5-5"]),
      provider("openai", "inKeychain", ["gpt-5"]),
      provider("openrouter", "missing", ["anthropic/claude-sonnet-5"]),
      provider("ollama", "notNeeded", ["qwen3-coder:30b"]),
    ]);
    expect(choices.map((c) => `${c.selection.provider}/${c.selection.model}`)).toEqual([
      "anthropic/claude-sonnet-5",
      "anthropic/claude-opus-5-5",
      "ollama/qwen3-coder:30b",
    ]);
  });

  it("offers nothing for an agent without an adapter", () => {
    expect(modelChoices({ ...agent, providers: [] }, [provider("anthropic", "inKeychain", ["x"])])).toEqual([]);
  });
});

describe("providers store", () => {
  it("keeps the latest list and replaces one provider at a time", async () => {
    const calls: boolean[] = [];
    const store = new Providers({
      listProviders: async (checkLocal) => {
        calls.push(checkLocal);
        return { providers: [provider("anthropic", "missing", []), provider("ollama", "notNeeded", [])] };
      },
    });
    await store.load(true);
    store.replace(provider("anthropic", "inKeychain", []));
    expect(store.find("anthropic")?.credential).toBe("inKeychain");
    expect(store.find("ollama")?.credential).toBe("notNeeded");
    expect(calls).toEqual([true]);
  });
});
