import { describe, expect, it } from "vitest";

import type { CatalogItem } from "../contracts/generated/CatalogItem";
import { Catalog, filterCatalog, searchText } from "./catalog";

function item(id: string, change: Partial<CatalogItem> = {}): CatalogItem {
  return {
    id,
    type: "agent",
    name: id,
    displayName: id,
    description: "",
    catalogVersion: "1",
    softwareVersion: null,
    publisher: null,
    source: "builtin",
    capabilities: [],
    tags: [],
    status: "available",
    statusDetail: null,
    requirements: [],
    details: { kind: "agent", agent: id, executable: null, providers: [], mcp: false, skills: false },
    ...change,
  };
}

const items: CatalogItem[] = [
  item("agent.claude-code", {
    displayName: "Claude Code",
    description: "Anthropic's coding agent in the terminal",
    publisher: "Anthropic",
    status: "installed",
    capabilities: ["MCP servers (stdio)", "Skills"],
    tags: ["terminal"],
  }),
  item("agent.codex", { displayName: "Codex", publisher: "OpenAI", status: "unavailable" }),
  item("model.ollama.qwen3-coder:30b", {
    type: "model",
    displayName: "qwen3-coder:30b",
    status: "available",
    details: { kind: "model", provider: "ollama", providerName: "Ollama", model: "qwen3-coder:30b", source: "custom" },
  }),
  item("model.anthropic.claude-sonnet-5", {
    type: "model",
    displayName: "Claude Sonnet 5",
    status: "configured",
    details: { kind: "model", provider: "anthropic", providerName: "Anthropic", model: "claude-sonnet-5", source: "builtIn" },
  }),
  item("mcp.github", { type: "mcpServer", displayName: "GitHub", source: "userDefined", status: "configured", tags: ["git"] }),
  item("skill.tests-first", { type: "skill", displayName: "Tests first", description: "Write the failing test first", status: "installed" }),
];

const ids = (list: CatalogItem[]) => list.map((i) => i.id);

describe("catalog search", () => {
  it("matches name, description, publisher, provider, tags and capabilities, every word", () => {
    const all = { category: "all" as const, status: "any" as const };
    expect(ids(filterCatalog(items, { ...all, text: "claude" }))).toEqual(["agent.claude-code", "model.anthropic.claude-sonnet-5"]);
    expect(ids(filterCatalog(items, { ...all, text: "failing test" }))).toEqual(["skill.tests-first"]);
    expect(ids(filterCatalog(items, { ...all, text: "openai" }))).toEqual(["agent.codex"]);
    expect(ids(filterCatalog(items, { ...all, text: "ollama" }))).toEqual(["model.ollama.qwen3-coder:30b"]);
    expect(ids(filterCatalog(items, { ...all, text: "git" }))).toEqual(["mcp.github"]);
    expect(ids(filterCatalog(items, { ...all, text: "MCP STDIO" }))).toEqual(["agent.claude-code"]);
    expect(ids(filterCatalog(items, { ...all, text: "claude openai" }))).toEqual([]);
    expect(filterCatalog(items, { ...all, text: "  " })).toHaveLength(items.length);
    expect(searchText(items[0]!)).toContain("anthropic");
  });

  it("filters by category and by status", () => {
    const query = { text: "", status: "any" as const };
    expect(ids(filterCatalog(items, { ...query, category: "model" }))).toEqual([
      "model.ollama.qwen3-coder:30b",
      "model.anthropic.claude-sonnet-5",
    ]);
    expect(ids(filterCatalog(items, { ...query, category: "skill" }))).toEqual(["skill.tests-first"]);
    expect(ids(filterCatalog(items, { text: "", category: "all", status: "installed" }))).toEqual(["agent.claude-code", "skill.tests-first"]);
    expect(ids(filterCatalog(items, { text: "", category: "all", status: "configured" }))).toEqual([
      "model.anthropic.claude-sonnet-5",
      "mcp.github",
    ]);
    expect(filterCatalog(items, { text: "", category: "all", status: "usable" })).toHaveLength(4);
    expect(ids(filterCatalog(items, { text: "claude", category: "model", status: "usable" }))).toEqual(["model.anthropic.claude-sonnet-5"]);
  });
});

describe("Catalog", () => {
  it("keeps what the native side assembled and reports a failure", async () => {
    let fail = false;
    const catalog = new Catalog({
      listCatalog: async () => {
        if (fail) throw new Error("no catalog");
        return { items, warnings: ["catalog metadata for agent.gone matches nothing the app knows"] };
      },
    });
    expect(catalog.get().items).toBeNull();
    await catalog.load();
    expect(catalog.find("mcp.github")?.displayName).toBe("GitHub");
    expect(catalog.get().warnings).toHaveLength(1);
    fail = true;
    await catalog.load();
    expect(catalog.get()).toMatchObject({ error: "no catalog", loading: false });
    expect(catalog.get().items).toHaveLength(items.length);
  });
});
