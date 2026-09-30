import { describe, expect, it } from "vitest";

import type { AppInfo } from "../contracts/generated/AppInfo";
import type { SearchEvent } from "../contracts/generated/SearchEvent";
import type { TerminalEvent } from "../contracts/generated/TerminalEvent";
import type { TerminalInfo } from "../contracts/generated/TerminalInfo";
import { createNativeClient, type Invoke, type InvokeArgs, type InvokeOptions, SESSION_ID_HEADER } from "./client";
import { NativeError } from "./errors";

const info: AppInfo = { name: "x8ai Workspace", version: "0.1.0", os: "macos", arch: "aarch64", userName: "Ada Lovelace" };

interface Call {
  command: string;
  args: InvokeArgs | undefined;
  options: InvokeOptions | undefined;
}

/** A fake bridge that records invocations and exposes the channels it created. */
function bridge(respond: (call: Call) => unknown = () => undefined) {
  const calls: Call[] = [];
  const channels: Array<(message: unknown) => void> = [];
  const invoke: Invoke = async (command, args, options) => {
    const call = { command, args, options };
    calls.push(call);
    return respond(call);
  };
  const createChannel = (onMessage: (message: unknown) => void) => {
    channels.push(onMessage);
    return { channel: channels.length };
  };
  return { calls, channels, client: createNativeClient({ invoke, createChannel }) };
}

function rejectingWith(reason: unknown) {
  return createNativeClient({ invoke: () => Promise.reject(reason), createChannel: () => ({}) });
}

async function failure(promise: Promise<unknown>): Promise<NativeError> {
  const error = await promise.then(
    () => expect.unreachable("expected the call to fail"),
    (e: unknown) => e,
  );
  expect(error).toBeInstanceOf(NativeError);
  return error as NativeError;
}

describe("native client", () => {
  it("invokes the matching command and returns its result", async () => {
    const { calls, client } = bridge(() => info);

    await expect(client.getAppInfo()).resolves.toEqual(info);
    expect(calls.map((c) => c.command)).toEqual(["get_app_info"]);
  });

  it("preserves structured command errors", async () => {
    const client = rejectingWith({ code: "notFound", message: "no such session" });

    const error = await failure(client.getAppInfo());
    expect(error).toMatchObject({ command: "get_app_info", code: "notFound", message: "no such session" });
  });

  it("reports calls that never reached a command as ipc errors", async () => {
    // Tauri rejects with a plain string when a command is not granted to the window.
    const client = rejectingWith("get_app_info not allowed");

    const error = await failure(client.getAppInfo());
    expect(error).toMatchObject({ code: "ipc", message: "get_app_info not allowed" });
  });

  it("reports a missing bridge as an ipc error", async () => {
    const client = rejectingWith(new TypeError("window.__TAURI_INTERNALS__ is undefined"));

    const error = await failure(client.getAppInfo());
    expect(error.code).toBe("ipc");
  });

  it("does not trust unknown error codes", async () => {
    const client = rejectingWith({ code: "rootAccessGranted", message: "?" });

    const error = await failure(client.getAppInfo());
    expect(error.code).toBe("ipc");
  });
});

describe("native client terminal commands", () => {
  const created: TerminalInfo = { id: 7, program: "/bin/zsh", cwd: "/Users/me/project", ackBytes: 65536 };

  it("creates a session with a channel and splits output from events", async () => {
    const { calls, channels, client } = bridge(() => created);
    const output: Uint8Array[] = [];
    const events: TerminalEvent[] = [];

    const result = await client.createTerminal(
      { cols: 80, rows: 24 },
      { output: (data) => output.push(data), event: (event) => events.push(event) },
    );

    expect(result).toEqual(created);
    expect(calls[0]).toMatchObject({
      command: "terminal_create",
      args: { size: { cols: 80, rows: 24 }, events: { channel: 1 } },
    });

    const deliver = channels[0]!;
    deliver(new Uint8Array([104, 105]).buffer);
    deliver({ type: "exited", code: 0, signal: null });
    expect(output).toEqual([new Uint8Array([104, 105])]);
    expect(events).toEqual([{ type: "exited", code: 0, signal: null }]);
  });

  it("writes input as a raw body addressed by header", async () => {
    const { calls, client } = bridge();
    const bytes = new Uint8Array([0x03]);

    await client.writeTerminal(7, bytes);

    expect(calls[0]).toEqual({
      command: "terminal_write",
      args: bytes,
      options: { headers: { [SESSION_ID_HEADER]: "7" } },
    });
  });

  it("sends resize, ack and close with named arguments", async () => {
    const { calls, client } = bridge();

    await client.resizeTerminal(7, { cols: 100, rows: 30 });
    await client.ackTerminal(7, 65536);
    await client.closeTerminal(7);

    expect(calls.map(({ command, args }) => ({ command, args }))).toEqual([
      { command: "terminal_resize", args: { id: 7, size: { cols: 100, rows: 30 } } },
      { command: "terminal_ack", args: { id: 7, bytes: 65536 } },
      { command: "terminal_close", args: { id: 7 } },
    ]);
  });
});

describe("native client workspace commands", () => {
  it("streams search results on a channel until the search is over", async () => {
    const { calls, channels, client } = bridge();
    const events: SearchEvent[] = [];

    await client.search({ text: "TODO", caseSensitive: false }, (event) => events.push(event));
    channels[0]!({ type: "file", path: "a.txt", matches: [] });
    channels[0]!({ type: "done", files: 1, matches: 0, truncated: false, cancelled: false });

    expect(calls[0]).toMatchObject({
      command: "workspace_search",
      args: { query: { text: "TODO", caseSensitive: false }, events: { channel: 1 } },
    });
    expect(events.map((e) => e.type)).toEqual(["file", "done"]);
  });

  it("reopens, forgets and trusts with named arguments", async () => {
    const { calls, client } = bridge();

    await client.openRecentWorkspace("/Users/me/project", () => {});
    await client.forgetRecentWorkspace("/Users/me/old");
    await client.setWorkspaceTrust(true);
    await client.isTerminalBusy(3);

    expect(calls.map(({ command, args }) => ({ command, args }))).toEqual([
      { command: "workspace_open_recent", args: { root: "/Users/me/project", events: { channel: 1 } } },
      { command: "workspace_forget_recent", args: { root: "/Users/me/old" } },
      { command: "workspace_set_trust", args: { trusted: true } },
      { command: "terminal_is_busy", args: { id: 3 } },
    ]);
  });
});

describe("native client agent commands", () => {
  it("runs an agent session with a session channel and names it by id only", async () => {
    const created: TerminalInfo = { id: 9, program: "/Users/me/.local/bin/claude", cwd: "/Users/me/.x8ai/worktrees/p/claude-code-1", ackBytes: 65536 };
    const { calls, channels, client } = bridge(() => created);
    const output: Uint8Array[] = [];

    await expect(
      client.runAgentSession(3, { cols: 80, rows: 24 }, { output: (d) => output.push(d), event: () => {} }),
    ).resolves.toEqual(created);
    channels[0]!(new Uint8Array([1, 2]).buffer);

    expect(calls[0]).toMatchObject({
      command: "agent_run",
      args: { session: 3, size: { cols: 80, rows: 24 }, events: { channel: 1 } },
    });
    expect(output).toEqual([new Uint8Array([1, 2])]);
  });

  it("passes only ids for sessions, never paths for worktrees", async () => {
    const { calls, client } = bridge();
    await client.createAgentSession("claude-code", null, ["docs"], ["tests-first"]);
    await client.agentSessions();
    await client.stopAgentSession(3);
    await client.removeAgentSession(3, false);
    await client.agentChanges(3);
    await client.readAgentFile(3, "src/main.rs");
    expect(calls.map(({ command, args }) => ({ command, args }))).toEqual([
      { command: "agent_create_session", args: { agent: "claude-code", model: null, mcp: ["docs"], skills: ["tests-first"] } },
      { command: "agent_sessions", args: undefined },
      { command: "agent_stop", args: { session: 3 } },
      { command: "agent_remove", args: { session: 3, discard: false } },
      { command: "agent_changes", args: { session: 3 } },
      { command: "agent_read_file", args: { session: 3, path: "src/main.rs" } },
    ]);
  });

  it("lists, approves and revokes with named arguments", async () => {
    const { calls, client } = bridge();
    await client.listAgents(true);
    await client.requestAgentApproval("claude-code", { provider: "openrouter", model: "anthropic/claude-sonnet-5" }, [], []);
    await client.requestSessionApproval(4);
    await client.revokeAgentApproval("claude-code");
    expect(calls.map(({ command, args }) => ({ command, args }))).toEqual([
      { command: "agent_list", args: { refresh: true } },
      {
        command: "agent_request_approval",
        args: { agent: "claude-code", model: { provider: "openrouter", model: "anthropic/claude-sonnet-5" }, mcp: [], skills: [] },
      },
      { command: "agent_request_session_approval", args: { session: 4 } },
      { command: "agent_revoke", args: { agent: "claude-code" } },
    ]);
  });
});

describe("native client provider commands", () => {
  it("sends a key once, to save it, and has no way to read one back", async () => {
    const { calls, client } = bridge();
    await client.listProviders(true);
    await client.setProviderCredential("anthropic", "sk-x8ai-test-invalid");
    await client.removeProviderCredential("anthropic");
    await client.addProviderModel("ollama", "qwen3-coder:30b");
    await client.removeProviderModel("ollama", "qwen3-coder:30b");
    expect(calls.map(({ command, args }) => ({ command, args }))).toEqual([
      { command: "provider_list", args: { checkLocal: true } },
      { command: "provider_set_credential", args: { provider: "anthropic", key: "sk-x8ai-test-invalid" } },
      { command: "provider_remove_credential", args: { provider: "anthropic" } },
      { command: "provider_add_model", args: { provider: "ollama", model: "qwen3-coder:30b" } },
      { command: "provider_remove_model", args: { provider: "ollama", model: "qwen3-coder:30b" } },
    ]);
    const methods = Object.keys(client).filter((name) => /credential|key|secret/i.test(name));
    // Secrets can be saved and removed, provider keys and MCP secrets alike; never read.
    expect(methods.sort()).toEqual(["removeMcpSecret", "removeProviderCredential", "setMcpSecret", "setProviderCredential"]);
  });
});

describe("native client MCP commands", () => {
  it("sends a secret once, to save it, and names servers by id", async () => {
    const { calls, client } = bridge();
    const server = {
      name: "GitHub",
      description: "",
      transport: { kind: "stdio" as const, command: "npx", args: ["-y", "@modelcontextprotocol/server-github"] },
      env: [{ name: "GITHUB_PERSONAL_ACCESS_TOKEN", source: "secret" as const }],
      enabled: true,
      scope: "global" as const,
    };
    await client.listMcpServers();
    await client.addMcpServer(server);
    await client.updateMcpServer("github", server);
    await client.setMcpServerEnabled("github", false);
    await client.setMcpSecret("github", "GITHUB_PERSONAL_ACCESS_TOKEN", "ghp_test_invalid");
    await client.removeMcpSecret("github", "GITHUB_PERSONAL_ACCESS_TOKEN");
    await client.removeMcpServer("github");
    expect(calls.map(({ command, args }) => ({ command, args }))).toEqual([
      { command: "mcp_list", args: undefined },
      { command: "mcp_add", args: { server } },
      { command: "mcp_update", args: { id: "github", server } },
      { command: "mcp_set_enabled", args: { id: "github", enabled: false } },
      { command: "mcp_set_secret", args: { id: "github", name: "GITHUB_PERSONAL_ACCESS_TOKEN", value: "ghp_test_invalid" } },
      { command: "mcp_remove_secret", args: { id: "github", name: "GITHUB_PERSONAL_ACCESS_TOKEN" } },
      { command: "mcp_remove", args: { id: "github" } },
    ]);
    // Nothing reads a secret back, and nothing starts a server.
    const methods = Object.keys(client).filter((name) => /mcp/i.test(name));
    expect(methods.filter((m) => /get|read|start|run|test/i.test(m))).toEqual([]);
  });
});

describe("native client skill and catalog commands", () => {
  it("names skills by id and only lists the catalog", async () => {
    const { calls, client } = bridge();
    const skill = { name: "Tests first", description: "", instructions: "Write the test first.", allowedTools: [], scope: "session" as const };
    await client.listSkills();
    await client.addSkill(skill);
    await client.updateSkill("tests-first", skill);
    await client.removeSkill("tests-first");
    await client.listCatalog();
    expect(calls.map(({ command, args }) => ({ command, args }))).toEqual([
      { command: "skill_list", args: undefined },
      { command: "skill_add", args: { skill } },
      { command: "skill_update", args: { id: "tests-first", skill } },
      { command: "skill_remove", args: { id: "tests-first" } },
      { command: "catalog_list", args: undefined },
    ]);
    // The catalog can list and nothing else: it installs, starts and fetches nothing.
    expect(Object.keys(client).filter((name) => /catalog/i.test(name))).toEqual(["listCatalog"]);
    // A skill is text: nothing runs one.
    expect(Object.keys(client).filter((name) => /skill/i.test(name) && /run|start|exec|install/i.test(name))).toEqual([]);
  });
});

describe("native client folder commands", () => {
  it("passes where the picker starts, and closes the folder", async () => {
    const { calls, client } = bridge();
    await client.openWorkspace(() => {}, "~/projects/app");
    await client.openWorkspace(() => {});
    await client.closeWorkspace();
    expect(calls[0]).toMatchObject({ command: "workspace_open", args: { start: "~/projects/app" } });
    expect(calls[1]).toMatchObject({ command: "workspace_open", args: { start: null } });
    expect(calls[2]).toMatchObject({ command: "workspace_close", args: undefined });
  });
});
