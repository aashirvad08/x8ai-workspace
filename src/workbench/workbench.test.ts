import { describe, expect, it, vi } from "vitest";

import type { AgentChanges } from "../contracts/generated/AgentChanges";
import type { AgentSessionInfo } from "../contracts/generated/AgentSessionInfo";
import type { AgentStatus } from "../contracts/generated/AgentStatus";
import type { AppEvent } from "../contracts/generated/AppEvent";
import type { CatalogItem } from "../contracts/generated/CatalogItem";
import type { DirEntry } from "../contracts/generated/DirEntry";
import type { FileVersion } from "../contracts/generated/FileVersion";
import type { McpServerStatus } from "../contracts/generated/McpServerStatus";
import type { ModelSelection } from "../contracts/generated/ModelSelection";
import type { ProviderStatus } from "../contracts/generated/ProviderStatus";
import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import type { SearchEvent } from "../contracts/generated/SearchEvent";
import type { SessionId } from "../contracts/generated/SessionId";
import type { SkillStatus } from "../contracts/generated/SkillStatus";
import type { WorkspaceEvent } from "../contracts/generated/WorkspaceEvent";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import { basename, dirname, isWithin } from "../lib/paths";
import { type NativeClient, NativeError } from "../native";
import { Workbench } from "./workbench";

/** An in-memory workspace behind a fake native client. */
function fakeNative() {
  const files = new Map<string, { text: string; version: number }>([
    ["src/main.py", { text: "print('hi')\n", version: 1 }],
    ["README.md", { text: "# readme\n", version: 1 }],
  ]);
  const dirs = new Set(["", "src"]);
  let nextVersion = 10;
  const calls: string[] = [];
  const state = {
    files,
    dirs,
    calls,
    pickResult: { root: "/Users/me/project", name: "project", trusted: false } as WorkspaceInfo | null,
    recent: [] as RecentWorkspace[],
    trusted: new Set<string>(),
    /** What the native trust dialog answers. */
    confirmTrust: true,
    open: null as WorkspaceInfo | null,
    busy: new Set<SessionId>(),
    warnings: [] as string[],
    searchEvents: [] as SearchEvent[],
    /** Agents the native side approved, per workspace root. */
    approvals: new Set<string>(),
    /** What the native approval dialog answers. */
    allowAgent: true,
    /** Whether the open folder is a Git repository (agents get worktrees). */
    git: true,
    agentSessions: [] as AgentSessionInfo[],
    agentChanges: null as AgentChanges | null,
    removed: [] as { session: number; discard: boolean }[],
    providers: [
      {
        id: "anthropic",
        name: "Anthropic",
        description: "",
        hosting: "hosted",
        credential: "missing",
        local: null,
        models: [{ id: "claude-sonnet-5", provider: "anthropic", name: "Claude Sonnet 5", source: "builtIn", contextWindow: null }],
      },
      { id: "ollama", name: "Ollama", description: "", hosting: "local", credential: "notNeeded", local: null, models: [] },
    ] as ProviderStatus[],
    mcp: [] as McpServerStatus[],
    skills: [
      {
        skill: {
          id: "tests-first",
          name: "Tests first",
          description: "",
          version: 1,
          instructions: "Write the test first.",
          allowedTools: [],
          source: "builtin",
          scope: { kind: "session" },
        },
        agents: [{ agent: "claude-code", supported: true, reason: null }],
      },
    ] as SkillStatus[],
    catalog: [] as CatalogItem[],
    /** Agents listed after Claude Code. */
    moreAgents: [] as AgentStatus[],
    appListener: null as ((event: AppEvent) => void) | null,
    diskListener: null as ((event: WorkspaceEvent) => void) | null,
    unsaved: false,
    quit: false,
  };
  const remember = (info: WorkspaceInfo) => {
    state.recent = [{ root: info.root, name: info.name, available: true }, ...state.recent.filter((w) => w.root !== info.root)];
  };
  const notFound = (path: string) => new NativeError("x", "notFound", `"${path}" does not exist`);
  const native: NativeClient = {
    getAppInfo: async () => ({ name: "x8ai", version: "0", os: "macos", arch: "aarch64", userName: "Ada Lovelace" }),
    subscribeApp: async (listener) => void (state.appListener = listener),
    setUnsavedChanges: async (unsaved) => void (state.unsaved = unsaved),
    quit: async () => void (state.quit = true),
    takeWarnings: async () => state.warnings.splice(0),
    createTerminal: () => new Promise(() => {}),
    writeTerminal: async () => {},
    resizeTerminal: async () => {},
    ackTerminal: async () => {},
    closeTerminal: async () => {},
    isTerminalBusy: async (id) => state.busy.has(id),
    openWorkspace: async (listener, start) => {
      calls.push(`pick${start ? ` from ${start}` : ""}`);
      if (!state.pickResult) return null;
      state.diskListener = listener;
      const info = { ...state.pickResult, trusted: state.trusted.has(state.pickResult.root) };
      state.open = info;
      remember(info);
      return info;
    },
    closeWorkspace: async () => {
      calls.push("closeWorkspace");
      state.open = null;
    },
    openRecentWorkspace: async (root, listener) => {
      calls.push(`openRecent ${root}`);
      const entry = state.recent.find((w) => w.root === root);
      if (!entry) throw new NativeError("x", "permissionDenied", "not in Recent");
      if (!entry.available) {
        state.recent = state.recent.filter((w) => w.root !== root);
        throw new NativeError("x", "notFound", `${root} no longer exists, so it was removed from Recent`);
      }
      state.diskListener = listener;
      const info = { root, name: entry.name, trusted: state.trusted.has(root) };
      state.open = info;
      remember(info);
      return info;
    },
    recentWorkspaces: async () => state.recent,
    forgetRecentWorkspace: async (root) => void (state.recent = state.recent.filter((w) => w.root !== root)),
    setWorkspaceTrust: async (trusted) => {
      const open = state.open!;
      if (trusted && !state.confirmTrust) return open;
      if (trusted) state.trusted.add(open.root);
      else state.trusted.delete(open.root);
      state.open = { ...open, trusted };
      return state.open;
    },
    listDir: async (dir) => {
      if (!dirs.has(dir)) throw notFound(dir);
      const entries: DirEntry[] = [];
      for (const d of dirs) if (d !== "" && dirname(d) === dir) entries.push({ name: basename(d), path: d, kind: "directory", symlink: false });
      for (const f of files.keys()) if (dirname(f) === dir) entries.push({ name: basename(f), path: f, kind: "file", symlink: false });
      return entries;
    },
    readFile: async (path) => {
      const file = files.get(path);
      if (!file) throw notFound(path);
      return { text: file.text, version: String(file.version) };
    },
    fileVersion: async (path) => (files.has(path) ? String(files.get(path)!.version) : null),
    writeFile: async (path: string, text: string, expected: FileVersion | null) => {
      calls.push(`write ${path}`);
      const file = files.get(path);
      if (expected !== null && (!file || String(file.version) !== expected)) {
        throw new NativeError("workspace_write_file", "conflict", `"${path}" changed on disk`);
      }
      files.set(path, { text, version: nextVersion++ });
      return String(nextVersion - 1);
    },
    createFile: async (path) => {
      calls.push(`createFile ${path}`);
      if (files.has(path)) throw new NativeError("x", "alreadyExists", `"${path}" already exists`);
      files.set(path, { text: "", version: nextVersion++ });
    },
    createDir: async (path) => {
      calls.push(`createDir ${path}`);
      dirs.add(path);
    },
    renameEntry: async (from, to) => {
      calls.push(`rename ${from} ${to}`);
      const file = files.get(from);
      if (!file) throw notFound(from);
      files.delete(from);
      files.set(to, file);
    },
    deleteEntry: async (path) => {
      calls.push(`delete ${path}`);
      for (const f of [...files.keys()]) if (isWithin(f, path)) files.delete(f);
      dirs.delete(path);
    },
    listFiles: async () => ({ paths: [...files.keys()].sort(), truncated: false }),
    search: async (query, listener) => {
      calls.push(`search ${query.text}`);
      for (const event of state.searchEvents) listener(event);
    },
    cancelSearch: async () => void calls.push("cancelSearch"),
    listAgents: async () => ({
      agents: [
        {
          id: "claude-code",
          name: "Claude Code",
          description: "",
          availability: { state: "installed", executable: "/Users/me/.local/bin/claude" },
          approved: state.approvals.has(`${state.open?.root}:claude-code`),
          providers: [
            { provider: "anthropic", supported: true, reason: null },
            { provider: "ollama", supported: true, reason: null },
          ],
          mcp: { supported: true, reason: null },
          skills: { supported: true, reason: null },
          capabilities: { modelApis: ["anthropicMessages"], mcpTransports: ["stdio"] },
        },
        ...state.moreAgents,
      ],
      environmentProblem: null,
      isolation: state.git
        ? { kind: "worktrees", branch: "main", head: "a".repeat(40) }
        : { kind: "unavailable", reason: "This folder is not a Git repository." },
    }),
    requestAgentApproval: async (agent, model, mcp, skills) => {
      calls.push(`approve ${agent}${describeLaunch(model, mcp, skills)}`);
      const open = state.open!;
      if (!state.trusted.has(open.root)) throw new NativeError("x", "permissionDenied", "not trusted");
      const key = `${open.root}:${agent}${model ? `@${model.provider}` : ""}`;
      if (!state.approvals.has(key) && state.allowAgent) state.approvals.add(key);
      return state.approvals.has(key);
    },
    revokeAgentApproval: async (agent) => void state.approvals.delete(`${state.open?.root}:${agent}`),
    requestSessionApproval: async (session) => {
      calls.push(`approveSession ${session}`);
      return state.allowAgent;
    },
    createAgentSession: async (agent, model, mcp, skills) => {
      calls.push(`createSession ${agent}${describeLaunch(model, mcp, skills)}`);
      const open = state.open!;
      if (!state.git && state.agentSessions.some((s) => !s.worktree && s.state.state === "running")) {
        throw new NativeError("x", "conflict", "this folder is not a Git repository, so agents cannot get workspaces of their own");
      }
      const id = state.agentSessions.length + 1;
      const branch = `agent/${agent}/2026092${id}-101500-abcdef`;
      const session: AgentSessionInfo = {
        id,
        agent,
        name: "Claude Code",
        workspace: open.root,
        cwd: state.git ? `/Users/me/.x8ai/worktrees/project-1/${agent}-${id}` : open.root,
        worktree: state.git ? { branch, base: "a".repeat(40), path: `/Users/me/.x8ai/worktrees/project-1/${agent}-${id}` } : null,
        startedAt: 0,
        state: { state: "notRunning" },
        terminal: null,
        configuration: model
          ? {
              source: "app",
              provider: model.provider,
              providerName: "Anthropic",
              model: model.model,
              endpoint: "https://api.anthropic.com",
              credential: "inKeychain",
              overriddenShellVariables: ["ANTHROPIC_API_KEY"],
            }
          : { source: "agent", shellVariables: [] },
        mcp: mcp.map((id) => ({ id, name: id, transport: "stdio", state: { state: "idle" } })),
        skills: skills.map((id) => ({ id, name: id, version: 1, state: { state: "attached" } })),
      };
      state.agentSessions.push(session);
      return session;
    },
    runAgentSession: () => new Promise(() => {}),
    agentSessions: async () => state.agentSessions.map((s) => ({ ...s })),
    stopAgentSession: async (session) => {
      calls.push(`stop ${session}`);
      const found = state.agentSessions.find((s) => s.id === session);
      if (found) found.state = { state: "notRunning" };
    },
    removeAgentSession: async (session, discard) => {
      state.removed.push({ session, discard });
      state.agentSessions = state.agentSessions.filter((s) => s.id !== session);
      return { keptBranch: null, commits: 0 };
    },
    agentChanges: async () => state.agentChanges!,
    readAgentFile: async (session, path) => {
      calls.push(`readAgentFile ${session} ${path}`);
      return { text: `agent's ${path}\n`, version: "1" };
    },
    listProviders: async (checkLocal) => {
      calls.push(`listProviders ${checkLocal}`);
      return { providers: state.providers.map((p) => ({ ...p })) };
    },
    setProviderCredential: async (provider, key) => {
      if (key.includes("\n")) throw new NativeError("provider_set_credential", "invalidInput", "the key contains control characters");
      return changeProvider(provider, (p) => ({ ...p, credential: "inKeychain" }));
    },
    removeProviderCredential: async (provider) => changeProvider(provider, (p) => ({ ...p, credential: "missing" })),
    addProviderModel: async (provider, model) => {
      if (model.startsWith("-")) throw new NativeError("provider_add_model", "invalidInput", "not a model id");
      return changeProvider(provider, (p) => ({
        ...p,
        models: [...p.models, { id: model, provider, name: model, source: "custom", contextWindow: null }],
      }));
    },
    removeProviderModel: async (provider, model) =>
      changeProvider(provider, (p) => ({ ...p, models: p.models.filter((m) => m.id !== model) })),
    listMcpServers: async () => {
      calls.push("listMcp");
      return { servers: state.mcp.map((s) => ({ ...s })) };
    },
    addMcpServer: async (server) => {
      if (server.transport.kind === "stdio" && server.transport.command.includes(" ")) {
        throw new NativeError("mcp_add", "invalidInput", "transport.command: must be one program name; put its arguments in the argument list");
      }
      const status: McpServerStatus = {
        server: { ...server, id: server.name.toLowerCase(), scope: { kind: server.scope === "workspace" ? "workspace" : server.scope, root: state.open?.root ?? "" } as McpServerStatus["server"]["scope"] },
        secrets: server.env.filter((v) => v.source === "secret").map((v) => ({ name: v.name, state: "missing" })),
        configured: server.env.every((v) => v.source !== "secret"),
        problem: null,
        agents: [{ agent: "claude-code", supported: true, reason: null }],
      };
      state.mcp.push(status);
      return status;
    },
    updateMcpServer: async (id, server) => changeMcp(id, (s) => ({ ...s, server: { ...s.server, name: server.name } })),
    setMcpServerEnabled: async (id, enabled) => changeMcp(id, (s) => ({ ...s, server: { ...s.server, enabled } })),
    removeMcpServer: async (id) => {
      calls.push(`removeMcp ${id}`);
      state.mcp = state.mcp.filter((s) => s.server.id !== id);
    },
    setMcpSecret: async (id, name, value) => {
      if (value.includes("\n")) throw new NativeError("mcp_set_secret", "invalidInput", "the key contains control characters");
      return changeMcp(id, (s) => ({ ...s, secrets: s.secrets.map((x) => (x.name === name ? { ...x, state: "inKeychain" } : x)), configured: true }));
    },
    removeMcpSecret: async (id, name) =>
      changeMcp(id, (s) => ({ ...s, secrets: s.secrets.map((x) => (x.name === name ? { ...x, state: "missing" } : x)), configured: false })),
    listSkills: async () => {
      calls.push("listSkills");
      return { skills: state.skills.map((s) => ({ ...s })) };
    },
    addSkill: async (input) => {
      if (/(^|\s)sk-\S{8,}/.test(input.instructions)) {
        throw new NativeError("skill_add", "invalidInput", "instructions: looks like it contains a credential; skills must not hold secrets");
      }
      const skill = {
        ...input,
        id: input.name.toLowerCase().replaceAll(" ", "-"),
        version: 1,
        source: "user" as const,
        scope: input.scope === "workspace" ? { kind: "workspace" as const, root: state.open?.root ?? "" } : { kind: input.scope },
      } as SkillStatus["skill"];
      calls.push(`addSkill ${skill.id}`);
      state.skills.push({ skill, agents: [{ agent: "claude-code", supported: true, reason: null }] });
      return skill;
    },
    updateSkill: async (id, input) => {
      calls.push(`updateSkill ${id}`);
      const found = state.skills.find((s) => s.skill.id === id)!;
      found.skill = { ...found.skill, name: input.name, instructions: input.instructions, version: found.skill.version + 1 };
      return found.skill;
    },
    removeSkill: async (id) => {
      calls.push(`removeSkill ${id}`);
      state.skills = state.skills.filter((s) => s.skill.id !== id);
    },
    listCatalog: async () => {
      calls.push("listCatalog");
      return { items: state.catalog.map((i) => ({ ...i })), warnings: [] };
    },
  };
  function changeMcp(id: string, change: (s: McpServerStatus) => McpServerStatus): McpServerStatus {
    state.mcp = state.mcp.map((s) => (s.server.id === id ? change(s) : s));
    return state.mcp.find((s) => s.server.id === id)!;
  }
  function changeProvider(id: string, change: (p: ProviderStatus) => ProviderStatus): ProviderStatus {
    state.providers = state.providers.map((p) => (p.id === id ? change(p) : p));
    return state.providers.find((p) => p.id === id)!;
  }
  return { native, state };
}

function describeLaunch(model: ModelSelection | null, mcp: readonly string[], skills: readonly string[]): string {
  return `${model ? ` ${model.provider}/${model.model}` : ""}${mcp.length ? ` mcp:${mcp.join(",")}` : ""}${skills.length ? ` skills:${skills.join(",")}` : ""}`;
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

async function opened() {
  const { native, state } = fakeNative();
  const workbench = new Workbench(native);
  await workbench.start();
  await workbench.openFolder();
  await settle();
  return { workbench, state };
}

/** Pretends the pane's shell started, as its view reports once the native session exists. */
function started(workbench: Workbench, pane: number, id: SessionId) {
  workbench.terminals.started(pane, { id, program: "/bin/zsh", cwd: "/Users/me/project", ackBytes: 65536 });
}

/** Answers the dialog that is showing, or fails if none is. */
async function answer(workbench: Workbench, value: string) {
  await settle();
  const dialog = workbench.dialogs.get();
  expect(dialog, "a dialog should be showing").not.toBeNull();
  workbench.dialogs.answer(value);
  await settle();
}

function type(workbench: Workbench, path: string, text: string) {
  const state = workbench.editor.stateOf(path)!;
  workbench.editor.applyViewState(path, state.update({ changes: { from: 0, insert: text } }).state);
}

describe("Workbench", () => {
  it("opens a folder: explorer, and a terminal that starts there", async () => {
    const { workbench } = await opened();
    expect(workbench.workspace.get()).toEqual({ root: "/Users/me/project", name: "project", trusted: false });
    expect(workbench.explorer.get().listings.get("")!.entries!.map((e) => e.name)).toEqual(["src", "README.md"]);
    // The first terminal (started in the home directory) stays; a new one opens.
    expect(workbench.terminals.get().tabs).toHaveLength(2);
  });

  it("keeps the current workspace when the folder picker is cancelled", async () => {
    const { workbench, state } = await opened();
    state.pickResult = null;
    await workbench.openFolder();
    expect(workbench.workspace.get()?.name).toBe("project");
    expect(workbench.terminals.get().tabs).toHaveLength(2);
  });

  it("creates a file and opens it", async () => {
    const { workbench, state } = await opened();
    expect(await workbench.create("src", "train.py", "file")).toBe(true);
    await settle();
    expect(state.calls).toContain("createFile src/train.py");
    expect(workbench.editor.get().active).toBe("src/train.py");
    expect(workbench.explorer.get().selected).toBe("src/train.py");
  });

  it("creates inside the selected folder, even one never expanded", async () => {
    const { workbench } = await opened();
    workbench.explorer.select("src");
    workbench.startCreating("file");
    expect(workbench.explorer.get().editing).toEqual({ kind: "newFile", parent: "src" });
    workbench.explorer.select("README.md");
    workbench.startCreating("folder");
    expect(workbench.explorer.get().editing).toEqual({ kind: "newFolder", parent: "" });
  });

  it("shows why a create failed", async () => {
    const { workbench } = await opened();
    expect(await workbench.create("", "README.md", "file")).toBe(false);
    expect(workbench.notifications.get()[0]).toMatchObject({ tone: "error" });
    expect(workbench.notifications.get()[0]!.message).toContain("already exists");
  });

  it("shows why a file could not be opened", async () => {
    const { workbench } = await opened();
    workbench.openFile("missing.txt");
    await settle();
    expect(workbench.notifications.get()[0]).toMatchObject({ tone: "error" });
    expect(workbench.notifications.get()[0]!.message).toContain("does not exist");
    expect(workbench.editor.get().tabs).toHaveLength(0);
  });

  it("asks before closing a tab with unsaved changes", async () => {
    const { workbench } = await opened();
    workbench.openFile("README.md");
    await settle();
    type(workbench, "README.md", "edit ");

    const closing = workbench.closeEditor("README.md");
    await answer(workbench, "cancel");
    expect(await closing).toBe(false);
    expect(workbench.editor.isOpen("README.md")).toBe(true);

    const discarding = workbench.closeEditor("README.md");
    await answer(workbench, "discard");
    expect(await discarding).toBe(true);
    expect(workbench.editor.isOpen("README.md")).toBe(false);
  });

  it("saves before closing when asked to", async () => {
    const { workbench, state } = await opened();
    workbench.openFile("README.md");
    await settle();
    type(workbench, "README.md", "edit ");
    const closing = workbench.closeEditor("README.md");
    await answer(workbench, "save");
    expect(await closing).toBe(true);
    expect(state.files.get("README.md")!.text).toBe("edit # readme\n");
  });

  it("reports a save conflict and offers to overwrite", async () => {
    const { workbench, state } = await opened();
    workbench.openFile("README.md");
    await settle();
    type(workbench, "README.md", "mine ");
    state.files.set("README.md", { text: "theirs", version: 99 });

    workbench.save("README.md");
    await settle();
    const [notification] = workbench.notifications.get();
    expect(notification!.message).toContain("changed on disk");
    expect(state.files.get("README.md")!.text).toBe("theirs");

    notification!.actions.find((a) => a.label === "Overwrite")!.run();
    await settle();
    expect(state.files.get("README.md")!.text).toBe("mine # readme\n");
  });

  it("renames an open file and its tab follows", async () => {
    const { workbench } = await opened();
    workbench.openFile("src/main.py");
    await settle();
    expect(await workbench.rename("src/main.py", "app.py")).toBe(true);
    expect(workbench.editor.get().tabs[0]).toMatchObject({ path: "src/app.py", name: "app.py" });
  });

  it("moves deleted entries to the Trash only after confirming", async () => {
    const { workbench, state } = await opened();
    workbench.openFile("README.md");
    await settle();
    const entry: DirEntry = { name: "README.md", path: "README.md", kind: "file", symlink: false };

    workbench.remove(entry);
    await answer(workbench, "cancel");
    expect(state.calls).not.toContain("delete README.md");

    workbench.remove(entry);
    await answer(workbench, "delete");
    expect(state.calls).toContain("delete README.md");
    expect(workbench.editor.get().tabs[0]).toMatchObject({ path: "README.md", disk: "deleted" });
  });

  it("tells the native side about unsaved changes, and asks before quitting", async () => {
    const { workbench, state } = await opened();
    workbench.openFile("README.md");
    await settle();
    type(workbench, "README.md", "edit ");
    expect(state.unsaved).toBe(true);

    state.appListener!({ type: "quitRequested" });
    await answer(workbench, "cancel");
    expect(state.quit).toBe(false);

    state.appListener!({ type: "quitRequested" });
    await answer(workbench, "save");
    expect(state.files.get("README.md")!.text).toBe("edit # readme\n");
    expect(state.unsaved).toBe(false);
    expect(state.quit).toBe(true);
  });

  it("quits at once when nothing is unsaved", async () => {
    const { workbench, state } = await opened();
    state.appListener!({ type: "quitRequested" });
    await settle();
    expect(workbench.dialogs.get()).toBeNull();
    expect(state.quit).toBe(true);
  });

  it("follows changes on disk", async () => {
    const { workbench, state } = await opened();
    workbench.openFile("README.md");
    await settle();
    state.files.set("README.md", { text: "# from git\n", version: 50 });
    state.files.set("new.txt", { text: "", version: 51 });
    state.diskListener!({ type: "changed", paths: ["README.md", "new.txt"] });
    await settle();
    await settle();
    expect(workbench.editor.stateOf("README.md")!.doc.toString()).toBe("# from git\n");
    expect(workbench.explorer.get().listings.get("")!.entries!.map((e) => e.name)).toContain("new.txt");
  });
});

describe("Workbench workspaces", () => {
  it("reopens the most recent folder at startup, and starts the first terminal there", async () => {
    const { native, state } = fakeNative();
    state.recent = [{ root: "/Users/me/project", name: "project", available: true }];
    const workbench = new Workbench(native);
    await workbench.start();
    expect(state.calls).toContain("openRecent /Users/me/project");
    expect(workbench.workspace.get()?.root).toBe("/Users/me/project");
    expect(workbench.terminals.get().tabs).toHaveLength(1);
  });

  it("does not try to reopen a folder that is gone", async () => {
    const { native, state } = fakeNative();
    state.recent = [{ root: "/Volumes/usb/project", name: "project", available: false }];
    const workbench = new Workbench(native);
    await workbench.start();
    expect(state.calls.some((c) => c.startsWith("openRecent"))).toBe(false);
    expect(workbench.workspace.get()).toBeNull();
    expect(workbench.recent.get()).toEqual(state.recent);
    expect(workbench.terminals.get().tabs).toHaveLength(1);
  });

  it("reports a recent folder that disappeared, which then leaves the list", async () => {
    const { workbench, state } = await opened();
    state.recent.push({ root: "/Users/me/old", name: "old", available: false });
    workbench.openRecent("/Users/me/old");
    await settle();
    await settle();
    expect(workbench.notifications.get().at(-1)!.message).toContain("no longer exists");
    expect(workbench.recent.get().map((w) => w.root)).toEqual(["/Users/me/project"]);
    expect(workbench.workspace.get()?.root).toBe("/Users/me/project");
  });

  it("forgets a recent folder", async () => {
    const { workbench } = await opened();
    workbench.forgetRecent("/Users/me/project");
    await settle();
    expect(workbench.recent.get()).toEqual([]);
  });

  it("shows warnings from the native side", async () => {
    const { native, state } = fakeNative();
    state.warnings.push("recent-workspaces.json could not be read");
    const workbench = new Workbench(native);
    await workbench.start();
    expect(workbench.notifications.get().map((n) => n.message)).toContain("recent-workspaces.json could not be read");
  });

  it("keeps following the open folder's changes when the picker is cancelled", async () => {
    const { workbench, state } = await opened();
    state.pickResult = null;
    await workbench.openFolder();
    state.files.set("new.txt", { text: "", version: 51 });
    state.diskListener!({ type: "changed", paths: ["new.txt"] });
    await settle();
    await settle();
    expect(workbench.explorer.get().listings.get("")!.entries!.map((e) => e.name)).toContain("new.txt");
  });

  it("trusts a folder only once the native dialog confirms", async () => {
    const { workbench, state } = await opened();
    state.confirmTrust = false;
    await workbench.setTrust(true);
    expect(workbench.workspace.get()?.trusted).toBe(false);

    state.confirmTrust = true;
    await workbench.setTrust(true);
    expect(workbench.workspace.get()?.trusted).toBe(true);
  });

  it("asks before removing trust", async () => {
    const { workbench, state } = await opened();
    await workbench.setTrust(true);

    const declined = workbench.setTrust(false);
    await answer(workbench, "cancel");
    await declined;
    expect(state.trusted.has("/Users/me/project")).toBe(true);

    const removing = workbench.setTrust(false);
    await answer(workbench, "untrust");
    await removing;
    expect(workbench.workspace.get()?.trusted).toBe(false);
    expect(state.trusted.size).toBe(0);
  });
});

describe("Workbench search", () => {
  it("shows results and opens a match with it selected", async () => {
    const { workbench, state } = await opened();
    state.searchEvents = [
      { type: "file", path: "src/main.py", matches: [{ line: 1, column: 6, length: 4, preview: "print('hi')", ranges: [[6, 10]] }] },
      { type: "done", files: 1, matches: 1, truncated: false, cancelled: false },
    ];
    workbench.search.setText("'hi'");
    await workbench.search.run();
    const { status, results, summary } = workbench.search.get();
    expect(status).toBe("done");
    expect(results.map((r) => r.path)).toEqual(["src/main.py"]);
    expect(summary).toMatchObject({ files: 1, matches: 1 });

    workbench.openMatch("src/main.py", results[0]!.matches[0]!);
    await settle();
    expect(workbench.editor.get().active).toBe("src/main.py");
    const { from, to } = workbench.editor.stateOf("src/main.py")!.selection.main;
    expect([from, to]).toEqual([6, 10]);
  });

  it("clears results when another folder opens, and searches it again", async () => {
    const { workbench, state } = await opened();
    state.searchEvents = [
      { type: "file", path: "a.txt", matches: [] },
      { type: "done", files: 1, matches: 0, truncated: false, cancelled: false },
    ];
    workbench.search.setText("x");
    await workbench.search.run();
    state.searchEvents = [];
    state.pickResult = { root: "/Users/me/other", name: "other", trusted: false };
    await workbench.openFolder();
    await settle();
    expect(workbench.search.get().results).toEqual([]);
    expect(state.calls.filter((c) => c === "search x")).toHaveLength(2);
  });
});

describe("Workbench terminals", () => {
  it("splits the focused terminal into a new pane", async () => {
    const { workbench } = await opened();
    workbench.splitTerminal("right");
    const tab = workbench.terminals.activeTab()!;
    expect(tab.tree.kind).toBe("split");
    expect(workbench.terminals.get().panes.size).toBe(3);
  });

  it("asks before closing a pane that runs a program", async () => {
    const { workbench, state } = await opened();
    workbench.splitTerminal("down");
    const pane = workbench.terminals.activeTab()!.focused;
    started(workbench, pane, 5);
    state.busy.add(5);

    workbench.closeTerminalPane(pane);
    await answer(workbench, "cancel");
    expect(workbench.terminals.get().panes.has(pane)).toBe(true);

    workbench.closeTerminalPane(pane);
    await answer(workbench, "close");
    expect(workbench.terminals.get().panes.has(pane)).toBe(false);
    expect(workbench.terminals.activeTab()!.tree.kind).toBe("pane");
  });

  it("closes an idle shell without asking", async () => {
    const { workbench } = await opened();
    const tab = workbench.terminals.activeTab()!;
    started(workbench, tab.focused, 6);
    workbench.closeTerminalTab(tab.key);
    await settle();
    expect(workbench.dialogs.get()).toBeNull();
    expect(workbench.terminals.get().tabs).toHaveLength(1);
  });

  it("asks before quitting while a terminal runs a program", async () => {
    const { workbench, state } = await opened();
    started(workbench, workbench.terminals.activeTab()!.focused, 7);
    state.busy.add(7);

    state.appListener!({ type: "quitRequested" });
    await answer(workbench, "cancel");
    expect(state.quit).toBe(false);

    state.appListener!({ type: "quitRequested" });
    await answer(workbench, "quit");
    expect(state.quit).toBe(true);
  });
});

describe("Workbench agents", () => {
  const agentPanes = (workbench: Workbench) => workbench.terminals.agentPanes("claude-code");

  async function withAgentList() {
    const opened_ = await opened();
    await opened_.workbench.agents.load();
    return opened_;
  }

  it("does not start an agent in an untrusted folder unless the folder is trusted first", async () => {
    const { workbench, state } = await withAgentList();

    workbench.launchAgent("claude-code");
    await answer(workbench, "cancel");
    expect(state.calls).not.toContain("approve claude-code");
    expect(agentPanes(workbench)).toEqual([]);

    workbench.launchAgent("claude-code");
    await answer(workbench, "trust");
    await settle();
    expect(workbench.workspace.get()?.trusted).toBe(true);
    expect(state.calls).toContain("approve claude-code");
    expect(agentPanes(workbench)).toHaveLength(1);
  });

  it("does not start an agent the user did not allow", async () => {
    const { workbench, state } = await withAgentList();
    await workbench.setTrust(true);
    state.allowAgent = false;

    workbench.launchAgent("claude-code");
    await settle();
    await settle();
    expect(agentPanes(workbench)).toEqual([]);
    expect(workbench.agents.find("claude-code")?.approved).toBe(false);
  });

  it("starts an allowed agent in a terminal of its own", async () => {
    const { workbench } = await withAgentList();
    await workbench.setTrust(true);

    workbench.launchAgent("claude-code");
    await settle();
    await settle();
    const [pane] = agentPanes(workbench);
    expect(pane?.kind).toEqual({ type: "agent", agent: "claude-code", name: "Claude Code", session: 1 });
    expect(workbench.terminals.activeTab()?.focused).toBe(pane?.key);
    expect(workbench.agents.find("claude-code")?.approved).toBe(true);
  });

  it("asks before closing a running agent", async () => {
    const { workbench } = await withAgentList();
    await workbench.setTrust(true);
    workbench.launchAgent("claude-code");
    await settle();
    await settle();
    const pane = agentPanes(workbench)[0]!;
    started(workbench, pane.key, 11);

    workbench.closeTerminalPane(pane.key);
    await settle();
    expect(workbench.dialogs.get()?.title).toBe("Stop Claude Code?");
    await answer(workbench, "cancel");
    expect(agentPanes(workbench)).toHaveLength(1);

    workbench.closeTerminalPane(pane.key);
    await answer(workbench, "close");
    expect(agentPanes(workbench)).toEqual([]);
  });

  it("stops the agents when another folder opens, after asking", async () => {
    const { workbench, state } = await withAgentList();
    await workbench.setTrust(true);
    workbench.launchAgent("claude-code");
    await settle();
    await settle();
    started(workbench, agentPanes(workbench)[0]!.key, 12);
    state.pickResult = { root: "/Users/me/other", name: "other", trusted: false };

    const declined = workbench.openFolder();
    await answer(workbench, "cancel");
    await declined;
    expect(workbench.workspace.get()?.name).toBe("project");
    expect(agentPanes(workbench)).toHaveLength(1);

    const opening = workbench.openFolder();
    await answer(workbench, "stop");
    await opening;
    expect(workbench.workspace.get()?.name).toBe("other");
    expect(agentPanes(workbench)).toEqual([]);
  });

  it("closes the folder's agents when its trust is removed", async () => {
    const { workbench } = await withAgentList();
    await workbench.setTrust(true);
    workbench.launchAgent("claude-code");
    await settle();
    await settle();
    started(workbench, agentPanes(workbench)[0]!.key, 13);

    const removing = workbench.setTrust(false);
    await answer(workbench, "untrust");
    await removing;
    expect(agentPanes(workbench)).toEqual([]);
  });

  it("asks before quitting while an agent runs", async () => {
    const { workbench, state } = await withAgentList();
    await workbench.setTrust(true);
    workbench.launchAgent("claude-code");
    await settle();
    await settle();
    started(workbench, agentPanes(workbench)[0]!.key, 14);

    state.appListener!({ type: "quitRequested" });
    await settle();
    expect(workbench.dialogs.get()?.title).toBe("Quit and stop Claude Code?");
    await answer(workbench, "cancel");
    expect(state.quit).toBe(false);
  });
});

describe("Workbench agent sessions", () => {
  const agentPanes = (workbench: Workbench) => workbench.terminals.agentPanes("claude-code");

  async function trustedWithAgents() {
    const opened_ = await opened();
    await opened_.workbench.agents.load();
    await opened_.workbench.setTrust(true);
    return opened_;
  }

  async function launch(workbench: Workbench) {
    workbench.launchAgent("claude-code");
    for (let i = 0; i < 4; i++) await settle();
  }

  /** Marks a session running, as the native side reports once its agent started. */
  function running(state: ReturnType<typeof fakeNative>["state"], workbench: Workbench, id: number) {
    const session = state.agentSessions.find((s) => s.id === id)!;
    session.state = { state: "running" };
    const pane = workbench.terminals.paneOfSession(id)!;
    started(workbench, pane.key, 20 + id);
  }

  it("gives each launch a session of its own, in a Git repository at once", async () => {
    const { workbench, state } = await trustedWithAgents();
    await launch(workbench);
    running(state, workbench, 1);
    await launch(workbench);

    const panes = agentPanes(workbench);
    expect(panes.map((p) => (p.kind.type === "agent" ? p.kind.session : 0))).toEqual([1, 2]);
    expect(state.calls.filter((c) => c === "createSession claude-code")).toHaveLength(2);
    await workbench.agents.loadSessions();
    const [first, second] = workbench.agents.get().sessions;
    expect(first!.worktree!.path).not.toBe(second!.worktree!.path);
  });

  it("runs one agent at a time in a folder without Git, and says why", async () => {
    const { workbench, state } = await trustedWithAgents();
    state.git = false;
    await workbench.agents.load();
    expect(workbench.agents.get().isolation).toEqual({ kind: "unavailable", reason: "This folder is not a Git repository." });
    await launch(workbench);
    running(state, workbench, 1);

    await launch(workbench);
    expect(agentPanes(workbench)).toHaveLength(1);
    expect(workbench.notifications.get().at(-1)!.message).toContain("not a Git repository");
  });

  it("shows an agent's changes read-only, without touching the open folder", async () => {
    const { workbench, state } = await trustedWithAgents();
    await launch(workbench);
    await workbench.agents.loadSessions();
    state.agentChanges = {
      branch: "agent/claude-code/20260921-101500-abcdef",
      base: "a".repeat(40),
      head: "b".repeat(40),
      commits: 1,
      uncommitted: true,
      files: [{ path: "src/main.py", change: "modified", from: null }],
      diff: "--- a/src/main.py\n+++ b/src/main.py\n+print('agent')\n",
      truncated: false,
    };

    workbench.showAgentChanges(1);
    await settle();
    const diff = workbench.editor.get().tabs.find((t) => t.path === "/agent/1/changes.diff");
    expect(diff).toMatchObject({ readOnly: true, name: "Claude Code changes" });
    expect(workbench.editor.stateOf("/agent/1/changes.diff")!.doc.toString()).toContain("+print('agent')");
    expect(workbench.agents.get().changes.get(1)?.files).toHaveLength(1);

    workbench.openAgentFile(1, "src/main.py");
    await settle();
    const file = workbench.editor.get().tabs.find((t) => t.path === "/agent/1/src/main.py");
    expect(file).toMatchObject({ readOnly: true });
    expect(workbench.editor.stateOf("/agent/1/src/main.py")!.doc.toString()).toBe("agent's src/main.py\n");
    // Saving a read-only document writes nothing, and the workspace is the same.
    await workbench.editor.save("/agent/1/src/main.py");
    expect(state.calls.some((c) => c.startsWith("write "))).toBe(false);
    expect(workbench.workspace.get()?.root).toBe("/Users/me/project");
  });

  it("asks before stopping an agent, and keeps its session", async () => {
    const { workbench, state } = await trustedWithAgents();
    await launch(workbench);
    running(state, workbench, 1);
    await workbench.agents.loadSessions();

    workbench.stopAgent(1);
    await answer(workbench, "cancel");
    expect(state.calls).not.toContain("stop 1");

    workbench.stopAgent(1);
    await answer(workbench, "stop");
    workbench.terminals.ended(workbench.terminals.paneOfSession(1)!.key, { type: "exited", exit: { code: 0, signal: null } });
    for (let i = 0; i < 3; i++) await settle();
    expect(state.calls).toContain("stop 1");
    expect(workbench.agents.session(1)?.state).toEqual({ state: "notRunning" });
  });

  it("removes a workspace only after saying what goes, and discards uncommitted work only then", async () => {
    const { workbench, state } = await trustedWithAgents();
    await launch(workbench);
    await workbench.agents.loadSessions();
    state.agentChanges = {
      branch: "agent/claude-code/20260921-101500-abcdef",
      base: "a".repeat(40),
      head: "a".repeat(40),
      commits: 0,
      uncommitted: true,
      files: [{ path: "notes.txt", change: "untracked", from: null }],
      diff: "",
      truncated: false,
    };

    workbench.removeAgentSession(1);
    await answer(workbench, "cancel");
    expect(state.removed).toEqual([]);

    workbench.removeAgentSession(1);
    await settle();
    expect(workbench.dialogs.get()?.message).toContain("not committed are lost");
    await answer(workbench, "remove");
    expect(state.removed).toEqual([{ session: 1, discard: true }]);
    expect(agentPanes(workbench)).toEqual([]);
  });
});

describe("Workbench models", () => {
  it("looks for local providers when Models opens, not when Agents opens", async () => {
    const { workbench, state } = await opened();
    workbench.showAgents();
    await settle();
    expect(state.calls).toContain("listProviders false");
    expect(state.calls).not.toContain("listProviders true");
    workbench.showModels();
    await settle();
    expect(state.calls).toContain("listProviders true");
    expect(workbench.layout.get().sidebar).toBe("models");
  });

  it("saves a key without keeping it, and asks before removing it", async () => {
    const { workbench } = await opened();
    await workbench.providers.load();

    await expect(workbench.saveProviderKey("anthropic", "sk-x8ai-test-invalid")).resolves.toBe(true);
    expect(workbench.providers.find("anthropic")?.credential).toBe("inKeychain");
    // Neither the store nor any message holds the key.
    expect(JSON.stringify(workbench.providers.get())).not.toContain("sk-x8ai-test");
    expect(JSON.stringify(workbench.notifications.get())).not.toContain("sk-x8ai-test");

    workbench.removeProviderKey("anthropic");
    await answer(workbench, "cancel");
    expect(workbench.providers.find("anthropic")?.credential).toBe("inKeychain");
    workbench.removeProviderKey("anthropic");
    await answer(workbench, "remove");
    expect(workbench.providers.find("anthropic")?.credential).toBe("missing");
  });

  it("reports a refused key without repeating it", async () => {
    const { workbench } = await opened();
    await workbench.providers.load();
    await expect(workbench.saveProviderKey("anthropic", "sk-x8ai\nsecret")).resolves.toBe(false);
    const messages = workbench.notifications.get().map((n) => n.message);
    expect(messages).toEqual(["Could not save the Anthropic key: the key contains control characters"]);
    expect(workbench.providers.find("anthropic")?.credential).toBe("missing");
  });

  it("adds and removes model ids the user knows", async () => {
    const { workbench } = await opened();
    await workbench.providers.load();
    await expect(workbench.addProviderModel("ollama", "qwen3-coder:30b")).resolves.toBe(true);
    await expect(workbench.addProviderModel("ollama", "--help")).resolves.toBe(false);
    expect(workbench.providers.find("ollama")?.models.map((m) => m.id)).toEqual(["qwen3-coder:30b"]);
    workbench.removeProviderModel("ollama", "qwen3-coder:30b");
    await settle();
    expect(workbench.providers.find("ollama")?.models).toEqual([]);
  });

  it("launches an agent with a chosen model: approved for it, and kept by the session", async () => {
    const { workbench, state } = await opened();
    await workbench.agents.load();
    await workbench.setTrust(true);
    const model = { provider: "anthropic", model: "claude-sonnet-5" };

    workbench.launchAgent("claude-code", model);
    await settle();
    await settle();
    expect(state.calls).toContain("approve claude-code anthropic/claude-sonnet-5");
    expect(state.calls).toContain("createSession claude-code anthropic/claude-sonnet-5");
    await workbench.agents.loadSessions();
    expect(workbench.agents.session(1)?.configuration).toMatchObject({ source: "app", model: "claude-sonnet-5" });

    // The agent's own configuration is a separate approval.
    workbench.launchAgent("claude-code", null);
    await settle();
    await settle();
    expect(state.calls).toContain("approve claude-code");
    expect(state.approvals).toEqual(new Set(["/Users/me/project:claude-code@anthropic", "/Users/me/project:claude-code"]));
  });
});

describe("Workbench MCP servers", () => {
  const github = {
    name: "GitHub",
    description: "",
    transport: { kind: "stdio" as const, command: "npx", args: ["-y", "@modelcontextprotocol/server-github"] },
    env: [{ name: "GITHUB_PERSONAL_ACCESS_TOKEN", source: "secret" as const }],
    enabled: true,
    scope: "session" as const,
  };

  it("adds a server, saves its secret without keeping it, and asks before removing", async () => {
    const { workbench, state } = await opened();
    workbench.showMcp();
    await settle();
    expect(workbench.layout.get().sidebar).toBe("mcp");
    await expect(workbench.addMcpServer(github)).resolves.toBe(true);
    expect(workbench.mcp.find("github")?.configured).toBe(false);

    await expect(workbench.saveMcpSecret("github", "GITHUB_PERSONAL_ACCESS_TOKEN", "ghp_x8ai_test_invalid")).resolves.toBe(true);
    expect(workbench.mcp.find("github")?.secrets[0]?.state).toBe("inKeychain");
    expect(JSON.stringify(workbench.mcp.get())).not.toContain("ghp_x8ai");
    expect(JSON.stringify(workbench.notifications.get())).not.toContain("ghp_x8ai");

    workbench.removeMcpServer("github");
    await answer(workbench, "cancel");
    expect(workbench.mcp.find("github")).toBeDefined();
    workbench.removeMcpServer("github");
    await answer(workbench, "remove");
    expect(workbench.mcp.find("github")).toBeUndefined();
    expect(state.calls).toContain("removeMcp github");
  });

  it("reports a refused server or secret without repeating the secret", async () => {
    const { workbench } = await opened();
    await expect(workbench.addMcpServer({ ...github, transport: { kind: "stdio", command: "npx -y server", args: [] } })).resolves.toBe(false);
    await workbench.addMcpServer(github);
    await expect(workbench.saveMcpSecret("github", "GITHUB_PERSONAL_ACCESS_TOKEN", "ghp_x8ai\nsecret")).resolves.toBe(false);
    const messages = workbench.notifications.get().map((n) => n.message);
    expect(messages[0]).toContain("must be one program name");
    expect(messages.join(" ")).not.toContain("ghp_x8ai");
  });

  it("launches with chosen session servers: approved and created together", async () => {
    const { workbench, state } = await opened();
    await workbench.agents.load();
    await workbench.setTrust(true);
    workbench.launchAgent("claude-code", null, ["github"]);
    await settle();
    await settle();
    expect(state.calls).toContain("approve claude-code mcp:github");
    expect(state.calls).toContain("createSession claude-code mcp:github");
    await workbench.agents.loadSessions();
    expect(workbench.agents.session(1)?.mcp.map((s) => s.id)).toEqual(["github"]);
  });

  it("asks for approval again before an existing session runs again, and respects a no", async () => {
    const { workbench, state } = await opened();
    await workbench.agents.load();
    await workbench.setTrust(true);
    workbench.launchAgent("claude-code");
    await settle();
    await settle();
    const [pane] = workbench.terminals.agentPanes("claude-code");
    workbench.terminals.closePane(pane!.key);
    await workbench.agents.loadSessions();

    state.allowAgent = false;
    workbench.openAgentTerminal(1);
    await settle();
    expect(state.calls).toContain("approveSession 1");
    expect(workbench.terminals.agentPanes("claude-code")).toEqual([]);
    state.allowAgent = true;
    workbench.openAgentTerminal(1);
    await settle();
    expect(workbench.terminals.agentPanes("claude-code")).toHaveLength(1);
  });
});

describe("Workbench catalog", () => {
  const launches = (calls: readonly string[]) => calls.filter((c) => /^(approve|createSession)/.test(c));

  it("opens by listing, with nothing started, probed or approved", async () => {
    const { workbench, state } = await opened();
    // Starting the app and opening a folder neither lists the catalog nor looks for Ollama.
    expect(state.calls).not.toContain("listCatalog");
    expect(state.calls).not.toContain("listProviders true");
    state.calls.length = 0;
    workbench.showCatalog();
    await settle();
    expect(workbench.layout.get().sidebar).toBe("catalog");
    expect(state.calls).toContain("listCatalog");
    // The Ollama probe stays with Models, where the user asks for it.
    expect(state.calls).not.toContain("listProviders true");
    expect(launches(state.calls)).toEqual([]);
    workbench.refreshCatalog();
    await settle();
    expect(state.calls.filter((c) => c === "listCatalog")).toHaveLength(2);
    expect(workbench.commands().find((c) => c.id === "view.catalog")?.shortcut).toMatchObject({ key: "k", meta: true, shift: true });
  });

  it("chooses a model for the next launch without launching, and only a usable one", async () => {
    const { workbench, state } = await opened();
    workbench.chooseModel("anthropic", "claude-sonnet-5");
    await settle();
    expect(workbench.drafts.of("claude-code").model).toBeNull();
    expect(workbench.notifications.get().at(-1)?.message).toContain("No installed agent");

    state.providers[0] = { ...state.providers[0]!, credential: "inKeychain" };
    workbench.chooseModel("anthropic", "claude-sonnet-5");
    await settle();
    expect(workbench.drafts.of("claude-code").model).toEqual({ provider: "anthropic", model: "claude-sonnet-5" });
    expect(workbench.layout.get().sidebar).toBe("agents");
    expect(workbench.notifications.get().at(-1)?.message).toContain("Press Launch");
    expect(launches(state.calls)).toEqual([]);
  });

  it("attaches a session MCP server and a skill to the next launch, then launches only when asked", async () => {
    const { workbench, state } = await opened();
    await workbench.addMcpServer({
      name: "Echo",
      description: "",
      transport: { kind: "stdio", command: "python3", args: ["echo.py"] },
      env: [],
      enabled: true,
      scope: "session",
    });
    workbench.attachMcp("echo");
    workbench.attachSkill("tests-first");
    await settle();
    expect(workbench.drafts.of("claude-code")).toMatchObject({ mcp: ["echo"], skills: ["tests-first"] });
    expect(launches(state.calls)).toEqual([]);

    // A server that is not the user's to choose per session is not attached.
    workbench.attachMcp("missing");
    await settle();
    expect(workbench.drafts.of("claude-code").mcp).toEqual(["echo"]);

    await workbench.setTrust(true);
    const draft = workbench.drafts.of("claude-code");
    workbench.launchAgent("claude-code", draft.model, draft.mcp, draft.skills);
    await settle();
    await settle();
    expect(state.calls).toContain("approve claude-code mcp:echo skills:tests-first");
    expect(state.calls).toContain("createSession claude-code mcp:echo skills:tests-first");
    await workbench.agents.loadSessions();
    expect(workbench.agents.session(1)?.skills.map((s) => s.id)).toEqual(["tests-first"]);
  });

  it("adds, edits and removes the user's skills through the skill registry, asking before removing", async () => {
    const { workbench, state } = await opened();
    await expect(
      workbench.addSkill({ name: "Keys", description: "", instructions: "Use api_key sk-x8ai-test", allowedTools: [], scope: "session" }),
    ).resolves.toBe(false);
    expect(workbench.notifications.get().at(-1)?.message).toContain("must not hold secrets");
    expect(JSON.stringify(workbench.notifications.get())).not.toContain("sk-x8ai-test");

    const input = { name: "HEP analysis", description: "", instructions: "Use ROOT conventions.", allowedTools: [], scope: "session" as const };
    await expect(workbench.addSkill(input)).resolves.toBe(true);
    await settle();
    expect(workbench.skills.find("hep-analysis")?.skill.version).toBe(1);
    expect(state.calls).toContain("listCatalog");
    await expect(workbench.updateSkill("hep-analysis", { ...input, instructions: "Use ROOT 6 conventions." })).resolves.toBe(true);
    await settle();
    expect(workbench.skills.find("hep-analysis")?.skill.version).toBe(2);

    workbench.removeSkill("hep-analysis");
    await answer(workbench, "cancel");
    expect(state.calls).not.toContain("removeSkill hep-analysis");
    workbench.removeSkill("hep-analysis");
    await answer(workbench, "remove");
    await settle();
    expect(state.calls).toContain("removeSkill hep-analysis");
    expect(workbench.skills.find("hep-analysis")).toBeUndefined();
  });
});

describe("Workbench shortcuts", () => {
  it("gives every shortcut to one command", async () => {
    const { workbench } = await opened();
    const keys = workbench
      .commands()
      .filter((c) => c.shortcut)
      .map((c) => `${c.when ?? "any"} ${JSON.stringify(c.shortcut)}`);
    expect(new Set(keys).size).toBe(keys.length);
  });
});

describe("Workbench welcome", () => {
  const home = (workbench: Workbench) => workbench.home.get();

  it("starts on the welcome screen, with the last folder reopened behind it", async () => {
    const { native, state } = fakeNative();
    state.recent = [{ root: "/Users/me/project", name: "project", available: true }];
    const workbench = new Workbench(native);
    await workbench.start();
    await settle();
    expect(home(workbench).visible).toBe(true);
    expect(workbench.workspace.get()?.root).toBe("/Users/me/project");
    expect(workbench.home.name()).toBe("Ada Lovelace");

    workbench.leaveHome();
    expect(home(workbench).visible).toBe(false);
    const show = workbench.commands().find((c) => c.id === "view.home")!;
    expect(show.shortcut).toMatchObject({ key: "h", meta: true, shift: true });
    show.run();
    expect(home(workbench).visible).toBe(true);
    // Nothing typed: back to the workspace as it is.
    await workbench.runHomeCommand("   ");
    expect(home(workbench).visible).toBe(false);
  });

  it("/cd opens a recent space at once, and the welcome screen closes", async () => {
    const { workbench, state } = await opened();
    state.recent.push({ root: "/Users/me/other", name: "other", available: true });
    await workbench.start();
    workbench.showHome();
    await workbench.runHomeCommand("/cd other");
    expect(state.calls).toContain("openRecent /Users/me/other");
    expect(workbench.workspace.get()?.root).toBe("/Users/me/other");
    expect(home(workbench).visible).toBe(false);
    // The space already open: just back to it.
    workbench.showHome();
    await workbench.runHomeCommand("/cd /Users/me/other");
    expect(state.calls.filter((c) => c === "openRecent /Users/me/other")).toHaveLength(1);
    expect(home(workbench).visible).toBe(false);
  });

  it("/cd to a folder that is not a space yet opens the picker there, and only the user opens it", async () => {
    const { workbench, state } = await opened();
    workbench.showHome();
    state.pickResult = null;
    await workbench.runHomeCommand("/cd ~/projects/new");
    expect(state.calls).toContain("pick from ~/projects/new");
    expect(workbench.workspace.get()?.name).toBe("project");
    expect(home(workbench)).toMatchObject({ visible: true, message: { tone: "info" } });
    expect(home(workbench).message?.text).toContain("not one of your spaces yet");

    state.pickResult = { root: "/Users/me/projects/new", name: "new", trusted: false };
    await workbench.runHomeCommand("/cd ~/projects/new");
    expect(workbench.workspace.get()?.root).toBe("/Users/me/projects/new");
    expect(home(workbench).visible).toBe(false);

    // /cd alone is the picker from its usual place.
    workbench.showHome();
    await workbench.runHomeCommand("/cd");
    expect(state.calls.at(-1)).toBe("pick");
  });

  it("/cd that matches several spaces asks for more of the path", async () => {
    const { workbench, state } = await opened();
    state.recent.push({ root: "/Users/me/a/app", name: "app", available: true }, { root: "/Users/me/b/app", name: "app", available: true });
    await workbench.start();
    workbench.showHome();
    await workbench.runHomeCommand("/cd app");
    expect(home(workbench).message).toMatchObject({ tone: "error" });
    expect(home(workbench).message?.text).toContain("/Users/me/a/app, /Users/me/b/app");
    expect(workbench.workspace.get()?.name).toBe("project");
  });

  it("/home closes the folder: no folder, a shell at home", async () => {
    const { workbench, state } = await opened();
    workbench.showHome();
    const tabs = workbench.terminals.get().tabs.length;
    await workbench.runHomeCommand("/home");
    expect(state.calls).toContain("closeWorkspace");
    expect(workbench.workspace.get()).toBeNull();
    expect(workbench.explorer.get().listings.size).toBe(0);
    expect(workbench.terminals.get().tabs).toHaveLength(tabs + 1);
    expect(home(workbench).visible).toBe(false);
    // With no folder open, /home just shows the workspace.
    workbench.showHome();
    await workbench.runHomeCommand("/home");
    expect(state.calls.filter((c) => c === "closeWorkspace")).toHaveLength(1);
    expect(home(workbench).visible).toBe(false);
  });

  it("/home asks before stopping a running agent, and a no keeps the folder", async () => {
    const { workbench } = await opened();
    await workbench.agents.load();
    await workbench.setTrust(true);
    workbench.launchAgent("claude-code");
    await settle();
    await settle();
    started(workbench, workbench.terminals.agentPanes("claude-code")[0]!.key, 12);
    workbench.showHome();

    const declined = workbench.runHomeCommand("/home");
    await answer(workbench, "cancel");
    await declined;
    expect(workbench.workspace.get()?.name).toBe("project");
    expect(home(workbench).visible).toBe(true);

    const closing = workbench.runHomeCommand("/home");
    await answer(workbench, "stop");
    await closing;
    expect(workbench.workspace.get()).toBeNull();
    expect(workbench.terminals.agentPanes("claude-code")).toEqual([]);
  });

  it("/name changes the greeting, and anything else is explained", async () => {
    const { workbench } = await opened();
    workbench.showHome();
    await workbench.runHomeCommand("/name Countess of Lovelace");
    expect(workbench.home.name()).toBe("Countess of Lovelace");
    await workbench.runHomeCommand("/name");
    expect(workbench.home.name()).toBe("Ada Lovelace");
    await workbench.runHomeCommand("ls -la");
    expect(home(workbench).message).toMatchObject({ tone: "error" });
    expect(home(workbench).message?.text).toContain("/cd <folder>");
    expect(home(workbench).visible).toBe(true);
  });
});

describe("Workbench model drop", () => {
  const codex: AgentStatus = {
    id: "codex",
    name: "Codex",
    description: "",
    availability: { state: "installed", executable: "/opt/homebrew/bin/codex" },
    approved: false,
    providers: [{ provider: "openai", supported: true, reason: null }],
    mcp: { supported: false, reason: "no" },
    skills: { supported: false, reason: "no" },
    capabilities: { modelApis: ["openAiResponses"], mcpTransports: [] },
  };
  const openai: ProviderStatus = {
    id: "openai",
    name: "OpenAI",
    description: "",
    hosting: "hosted",
    credential: "inKeychain",
    local: null,
    models: [{ id: "gpt-6.1-sol", provider: "openai", name: "gpt-6.1-sol", source: "custom", contextWindow: null }],
  };

  it("opens a dropped model in the agent that can use it, through the usual approval", async () => {
    const { workbench, state } = await opened();
    state.moreAgents = [codex];
    state.providers.push(openai);
    await workbench.setTrust(true);
    workbench.launchModel("openai", "gpt-6.1-sol");
    await settle();
    await settle();
    await settle();
    expect(state.calls).toContain("approve codex openai/gpt-6.1-sol");
    expect(state.calls).toContain("createSession codex openai/gpt-6.1-sol");
    expect(workbench.notifications.get().some((n) => n.message.includes("in Codex"))).toBe(true);
  });

  it("says so when no installed agent can use it, and launches nothing", async () => {
    const { workbench, state } = await opened();
    state.providers.push(openai);
    workbench.launchModel("openai", "gpt-6.1-sol");
    await settle();
    await settle();
    expect(state.calls.filter((c) => c.startsWith("approve") || c.startsWith("createSession"))).toEqual([]);
    expect(workbench.notifications.get().at(-1)?.message).toContain("No installed agent here can use gpt-6.1-sol");
  });
});

describe("Workbench context between sessions", () => {
  function info(id: number, agent: string, name: string): AgentSessionInfo {
    return {
      id,
      agent,
      name,
      workspace: "/Users/me/project",
      cwd: `/Users/me/.x8ai/worktrees/project-1/${agent}-${id}`,
      worktree: { branch: `agent/${agent}/2026100${id}-101500-abcdef`, base: "a".repeat(40), path: `/w/${id}` },
      startedAt: 0,
      state: { state: "notRunning" },
      terminal: null,
      configuration: { source: "agent", shellVariables: [] },
      mcp: [],
      skills: [],
    };
  }

  /** A pane's terminal as the composer sees it: what it shows, and what was pasted. */
  function terminal(shown: string) {
    return {
      pasted: [] as string[],
      accepts: true,
      read: () => shown,
      acceptsPaste(): boolean {
        return this.accepts;
      },
      paste(text: string): boolean {
        if (!this.accepts) return false;
        this.pasted.push(text);
        return true;
      },
      quietFor: () => 5_000,
    };
  }

  /** Claude Code (session 1) and Codex (session 2), each running in its terminal unless `stopped`. */
  async function twoSessions(stopped: number[] = []) {
    const opened_ = await opened();
    const { workbench, state } = opened_;
    state.agentSessions = [info(1, "claude-code", "Claude Code"), info(2, "codex", "Codex")];
    state.agentChanges = {
      branch: "agent/claude-code/20261001-101500-abcdef",
      base: "a".repeat(40),
      head: "b".repeat(40),
      commits: 1,
      uncommitted: false,
      files: [{ path: "src/parser.rs", change: "added", from: null }],
      diff: "diff --git a/src/parser.rs b/src/parser.rs\n+++ b/src/parser.rs\n+fn parse() {}",
      truncated: false,
    };
    await workbench.agents.loadSessions();
    const terminals = new Map([
      [1, terminal("claude> wrote the parser\nAll tests pass.")],
      [2, terminal("codex>")],
    ]);
    for (const session of state.agentSessions.filter((s) => !stopped.includes(s.id))) {
      workbench.terminals.add({ type: "agent", agent: session.agent, name: session.name, session: session.id });
      const pane = workbench.terminals.paneOfSession(session.id)!;
      started(workbench, pane.key, 100 + session.id);
      workbench.terminals.setReader(pane.key, terminals.get(session.id)!);
    }
    return { ...opened_, terminals };
  }

  it("needs two sessions", async () => {
    const { workbench } = await opened();
    workbench.shareContext("get");
    await settle();
    expect(workbench.share.get()).toBeNull();
    expect(workbench.notifications.get().at(-1)?.message).toContain("start a second session first");
  });

  it("gets a session the others' context, and puts it in its input without pressing Enter", async () => {
    const { workbench, terminals } = await twoSessions();
    workbench.shareContext("get", 2);
    await settle();
    await settle();
    expect(workbench.share.get()).toMatchObject({ mode: "get", target: 2, sources: [1], edited: false });
    const text = workbench.share.get()!.text;
    expect(text).toContain("## Claude Code (agent/claude-code/20261001-101500-abcdef)");
    expect(text).toContain("- A src/parser.rs (+1 −0)");
    expect(text).toContain("claude> wrote the parser");
    expect(text).not.toContain("+fn parse() {}");

    workbench.setShareNote("Write the tests for it.");
    await settle();
    workbench.sendShareContext();
    await settle();
    await settle();
    const [pasted] = terminals.get(2)!.pasted;
    expect(pasted).toContain("Write the tests for it.");
    expect(pasted!.endsWith("\n")).toBe(false);
    expect(terminals.get(1)!.pasted).toEqual([]);
    expect(workbench.share.get()).toBeNull();
    expect(workbench.notifications.get().at(-1)?.message).toContain("in Codex's input");
  });

  it("gives a session's context to another, and sends the user's own edit as it is", async () => {
    const { workbench, terminals } = await twoSessions();
    workbench.shareContext("give", 1);
    await settle();
    await settle();
    expect(workbench.share.get()).toMatchObject({ mode: "give", sources: [1], target: 2 });
    workbench.setShareParts({ changes: true, diff: true, output: false });
    await settle();
    expect(workbench.share.get()!.text).toContain("+fn parse() {}");
    expect(workbench.share.get()!.text).not.toContain("wrote the parser");
    workbench.editShareText("Only this, \x1b[201~please.");
    expect(workbench.share.get()!.edited).toBe(true);
    workbench.sendShareContext();
    await settle();
    await settle();
    expect(terminals.get(2)!.pasted).toEqual(["Only this, please."]);
  });

  it("waits for the agent to take a paste, and sends nothing if it never does", async () => {
    const { workbench, terminals } = await twoSessions();
    workbench.shareContext("get", 2);
    await settle();
    await settle();
    terminals.get(2)!.accepts = false;
    vi.useFakeTimers();
    try {
      workbench.sendShareContext();
      await vi.advanceTimersByTimeAsync(50_000);
    } finally {
      vi.useRealTimers();
    }
    expect(terminals.get(2)!.pasted).toEqual([]);
    expect(workbench.share.get()).toMatchObject({ sending: false });
    expect(workbench.notifications.get().at(-1)?.message).toContain("nothing was sent");
  });

  it("starts a stopped agent only through its approval, then puts the context in", async () => {
    const { workbench, state, terminals } = await twoSessions([2]);
    workbench.shareContext("get", 2);
    await settle();
    await settle();

    // Refused: nothing starts, nothing is sent, the composer stays.
    state.allowAgent = false;
    workbench.sendShareContext();
    await answer(workbench, "start");
    await settle();
    expect(state.calls).toContain("approveSession 2");
    expect(workbench.terminals.paneOfSession(2)).toBeUndefined();
    expect(workbench.share.get()).toMatchObject({ sending: false });

    state.allowAgent = true;
    workbench.sendShareContext();
    await answer(workbench, "start");
    await settle();
    const pane = workbench.terminals.paneOfSession(2)!;
    expect(pane).toBeDefined();
    started(workbench, pane.key, 202);
    workbench.terminals.setReader(pane.key, terminals.get(2)!);
    await new Promise((resolve) => setTimeout(resolve, 450));
    expect(terminals.get(2)!.pasted).toHaveLength(1);
  });

  it("opens from the welcome screen with /get and /give", async () => {
    const { workbench } = await twoSessions();
    workbench.showHome();
    await workbench.runHomeCommand("/get codex");
    expect(workbench.home.get().visible).toBe(false);
    expect(workbench.share.get()).toMatchObject({ mode: "get", target: 2 });
    workbench.closeShareContext();

    workbench.showHome();
    await workbench.runHomeCommand("/give nobody");
    expect(workbench.home.get().message?.text).toContain("No agent session matches");
    expect(workbench.share.get()).toBeNull();
    await workbench.runHomeCommand("/give");
    expect(workbench.share.get()).toMatchObject({ mode: "give" });
    expect(workbench.commands().some((c) => c.id === "context.get")).toBe(true);
  });
});
