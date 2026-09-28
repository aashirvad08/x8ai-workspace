import { describe, expect, it } from "vitest";

import type { AppEvent } from "../contracts/generated/AppEvent";
import type { DirEntry } from "../contracts/generated/DirEntry";
import type { FileVersion } from "../contracts/generated/FileVersion";
import type { WorkspaceEvent } from "../contracts/generated/WorkspaceEvent";
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
    pickResult: { root: "/Users/me/project", name: "project" } as { root: string; name: string } | null,
    appListener: null as ((event: AppEvent) => void) | null,
    diskListener: null as ((event: WorkspaceEvent) => void) | null,
    unsaved: false,
    quit: false,
  };
  const notFound = (path: string) => new NativeError("x", "notFound", `"${path}" does not exist`);
  const native: NativeClient = {
    getAppInfo: async () => ({ name: "x8ai", version: "0", os: "macos", arch: "aarch64" }),
    subscribeApp: async (listener) => void (state.appListener = listener),
    setUnsavedChanges: async (unsaved) => void (state.unsaved = unsaved),
    quit: async () => void (state.quit = true),
    createTerminal: () => new Promise(() => {}),
    writeTerminal: async () => {},
    resizeTerminal: async () => {},
    ackTerminal: async () => {},
    closeTerminal: async () => {},
    openWorkspace: async (listener) => {
      state.diskListener = listener;
      return state.pickResult;
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
  };
  return { native, state };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

async function opened() {
  const { native, state } = fakeNative();
  const workbench = new Workbench(native);
  workbench.start();
  await workbench.openFolder();
  await settle();
  return { workbench, state };
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
    expect(workbench.workspace.get()).toEqual({ root: "/Users/me/project", name: "project" });
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
