import { describe, expect, it } from "vitest";

import type { AgentStatus } from "../contracts/generated/AgentStatus";
import type { McpServerStatus } from "../contracts/generated/McpServerStatus";
import { mcpChoices } from "./servers";

const claude: AgentStatus = {
  id: "claude-code",
  name: "Claude Code",
  description: "",
  availability: { state: "installed", executable: "/usr/local/bin/claude" },
  approved: false,
  providers: [],
  mcp: { supported: true, reason: null },
  skills: { supported: true, reason: null },
  capabilities: { modelApis: ["anthropicMessages"], mcpTransports: ["stdio"] },
};

function server(id: string, scope: McpServerStatus["server"]["scope"], enabled = true, supported = true): McpServerStatus {
  return {
    server: { id, name: id, description: "", transport: { kind: "stdio", command: "npx", args: [] }, env: [], enabled, scope },
    secrets: [],
    configured: true,
    problem: null,
    agents: [{ agent: "claude-code", supported, reason: supported ? null : "no" }],
  };
}

describe("MCP launch choices", () => {
  const servers = [
    server("global", { kind: "global" }),
    server("here", { kind: "workspace", root: "/Users/me/project" }),
    server("elsewhere", { kind: "workspace", root: "/Users/me/other" }),
    server("optional", { kind: "session" }),
    server("disabled", { kind: "global" }, false),
    server("unsupported", { kind: "global" }, true, false),
  ];

  it("attaches global and this folder's servers and offers session servers", () => {
    const choices = mcpChoices(claude, servers, "/Users/me/project");
    expect(choices.always.map((s) => s.server.id)).toEqual(["global", "here"]);
    expect(choices.optional.map((s) => s.server.id)).toEqual(["optional"]);
  });

  it("offers nothing to an agent without MCP support", () => {
    const choices = mcpChoices({ ...claude, mcp: { supported: false, reason: "no adapter" } }, servers, "/Users/me/project");
    expect(choices).toEqual({ always: [], optional: [] });
  });
});
