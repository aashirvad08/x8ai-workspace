import { describe, expect, it } from "vitest";

import type { AgentChanges } from "../contracts/generated/AgentChanges";
import type { AgentSessionInfo } from "../contracts/generated/AgentSessionInfo";
import type { AppEvent } from "../contracts/generated/AppEvent";
import type { DirEntry } from "../contracts/generated/DirEntry";
import type { FileVersion } from "../contracts/generated/FileVersion";
import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import type { SearchEvent } from "../contracts/generated/SearchEvent";
import type { SessionId } from "../contracts/generated/SessionId";
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
    getAppInfo: async () => ({ name: "x8ai", version: "0", os: "macos", arch: "aarch64" }),
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
    openWorkspace: async (listener) => {
      if (!state.pickResult) return null;
      state.diskListener = listener;
      const info = { ...state.pickResult, trusted: state.trusted.has(state.pickResult.root) };
      state.open = info;
      remember(info);
      return info;
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
        },
      ],
      environmentProblem: null,
      isolation: state.git
        ? { kind: "worktrees", branch: "main", head: "a".repeat(40) }
        : { kind: "unavailable", reason: "This folder is not a Git repository." },
    }),
    requestAgentApproval: async (agent) => {
      calls.push(`approve ${agent}`);
      const open = state.open!;
      if (!state.trusted.has(open.root)) throw new NativeError("x", "permissionDenied", "not trusted");
      const key = `${open.root}:${agent}`;
      if (!state.approvals.has(key) && state.allowAgent) state.approvals.add(key);
      return state.approvals.has(key);
    },
    revokeAgentApproval: async (agent) => void state.approvals.delete(`${state.open?.root}:${agent}`),
    createAgentSession: async (agent) => {
      calls.push(`createSession ${agent}`);
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
  };
  return { native, state };
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
