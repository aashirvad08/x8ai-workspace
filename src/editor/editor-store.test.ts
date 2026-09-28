import { describe, expect, it } from "vitest";

import type { FileVersion } from "../contracts/generated/FileVersion";
import { NativeError } from "../native";
import { EditorStore } from "./editor-store";

/** An in-memory disk. `version` bumps whenever a file changes. */
class FakeDisk {
  files = new Map<string, { text: string; version: number }>();
  writes: Array<{ path: string; text: string; expected: FileVersion | null }> = [];
  #next = 1;

  put(path: string, text: string): void {
    this.files.set(path, { text, version: this.#next++ });
  }
  delete(path: string): void {
    this.files.delete(path);
  }

  readonly native = {
    readFile: async (path: string) => {
      const file = this.files.get(path);
      if (!file) throw new NativeError("workspace_read_file", "notFound", `"${path}" does not exist`);
      return { text: file.text, version: String(file.version) };
    },
    fileVersion: async (path: string) => {
      const file = this.files.get(path);
      return file ? String(file.version) : null;
    },
    writeFile: async (path: string, text: string, expected: FileVersion | null) => {
      this.writes.push({ path, text, expected });
      const file = this.files.get(path);
      if (expected !== null && (!file || String(file.version) !== expected)) {
        throw new NativeError("workspace_write_file", "conflict", `"${path}" changed on disk`);
      }
      this.put(path, text);
      return String(this.files.get(path)!.version);
    },
  };
}

function setup() {
  const disk = new FakeDisk();
  disk.put("src/main.py", "print('hi')\n");
  disk.put("README.md", "# readme\n");
  return { disk, store: new EditorStore(disk.native) };
}

/** Types `insert` at the end of the document, as the view would. */
function type(store: EditorStore, path: string, insert: string) {
  const state = store.stateOf(path)!;
  store.applyViewState(path, state.update({ changes: { from: state.doc.length, insert } }).state);
}

const tab = (store: EditorStore, path: string) => store.get().tabs.find((t) => t.path === path)!;

describe("EditorStore", () => {
  it("opens files in tabs and switches between them", async () => {
    const { store } = setup();
    await store.open("src/main.py");
    await store.open("README.md");

    expect(store.get().tabs.map((t) => t.name)).toEqual(["main.py", "README.md"]);
    expect(store.get().active).toBe("README.md");
    expect(store.stateOf("src/main.py")!.doc.toString()).toBe("print('hi')\n");

    await store.open("src/main.py");
    expect(store.get().tabs).toHaveLength(2);
    expect(store.get().active).toBe("src/main.py");
  });

  it("opens a file only once when asked twice at the same time", async () => {
    const { store } = setup();
    await Promise.all([store.open("README.md"), store.open("README.md")]);
    expect(store.get().tabs).toHaveLength(1);
  });

  it("reports read errors to the caller", async () => {
    const { store } = setup();
    await expect(store.open("missing.txt")).rejects.toMatchObject({ code: "notFound" });
    expect(store.get().tabs).toHaveLength(0);
  });

  it("tracks unsaved changes, including undoing back to the saved text", async () => {
    const { store } = setup();
    await store.open("README.md");
    const saved = store.stateOf("README.md")!;

    type(store, "README.md", "more");
    expect(tab(store, "README.md").dirty).toBe(true);

    store.applyViewState("README.md", saved);
    expect(tab(store, "README.md").dirty).toBe(false);
  });

  it("saves against the version it read and becomes clean", async () => {
    const { disk, store } = setup();
    await store.open("README.md");
    type(store, "README.md", "more\n");

    await store.save("README.md");

    expect(disk.files.get("README.md")!.text).toBe("# readme\nmore\n");
    expect(disk.writes[0]!.expected).toBe("2");
    expect(tab(store, "README.md")).toMatchObject({ dirty: false, saving: false, disk: "synced" });
  });

  it("refuses to overwrite a file changed on disk, unless told to", async () => {
    const { disk, store } = setup();
    await store.open("README.md");
    type(store, "README.md", "mine\n");
    disk.put("README.md", "theirs\n");

    await expect(store.save("README.md")).rejects.toMatchObject({ code: "conflict" });
    expect(disk.files.get("README.md")!.text).toBe("theirs\n");
    expect(tab(store, "README.md")).toMatchObject({ dirty: true, saving: false });

    await store.save("README.md", { overwrite: true });
    expect(disk.files.get("README.md")!.text).toBe("# readme\nmine\n");
    expect(tab(store, "README.md").dirty).toBe(false);
  });

  it("keeps Windows line endings when saving", async () => {
    const { disk, store } = setup();
    disk.put("win.txt", "a\r\nb\r\n");
    await store.open("win.txt");
    type(store, "win.txt", "c");
    await store.save("win.txt");
    expect(disk.files.get("win.txt")!.text).toBe("a\r\nb\r\nc");
  });

  it("follows the disk for unmodified tabs", async () => {
    const { disk, store } = setup();
    await store.open("README.md");
    disk.put("README.md", "# updated elsewhere\n");

    await store.diskChanged(["README.md"]);
    expect(store.stateOf("README.md")!.doc.toString()).toBe("# updated elsewhere\n");
    expect(tab(store, "README.md")).toMatchObject({ dirty: false, disk: "synced" });
  });

  it("never touches unsaved edits when the disk changes, but marks the tab", async () => {
    const { disk, store } = setup();
    await store.open("README.md");
    type(store, "README.md", "mine");
    disk.put("README.md", "theirs");

    await store.diskChanged(["README.md"]);
    expect(store.stateOf("README.md")!.doc.toString()).toBe("# readme\nmine");
    expect(tab(store, "README.md").disk).toBe("changed");
  });

  it("recognises its own saves when the watcher reports them", async () => {
    const { store } = setup();
    await store.open("README.md");
    type(store, "README.md", "x");
    await store.save("README.md");
    await store.diskChanged(["README.md"]);
    expect(tab(store, "README.md")).toMatchObject({ disk: "synced", dirty: false });
  });

  it("marks tabs whose file, or a parent directory, was deleted", async () => {
    const { disk, store } = setup();
    await store.open("src/main.py");
    disk.delete("src/main.py");
    await store.diskChanged(["src"]);
    expect(tab(store, "src/main.py").disk).toBe("deleted");
  });

  it("moves tabs when a file or directory is renamed", async () => {
    const { store } = setup();
    await store.open("src/main.py");
    store.renamed("src", "lib");
    expect(store.get().tabs[0]).toMatchObject({ path: "lib/main.py", name: "main.py" });
    expect(store.get().active).toBe("lib/main.py");
    expect(store.stateOf("lib/main.py")).toBeDefined();
  });

  it("closes tabs and activates a neighbour", async () => {
    const { store } = setup();
    await store.open("src/main.py");
    await store.open("README.md");
    store.close("README.md");
    expect(store.get().active).toBe("src/main.py");
    store.close("src/main.py");
    expect(store.get()).toMatchObject({ tabs: [], active: null });
  });

  it("lists dirty paths", async () => {
    const { store } = setup();
    await store.open("src/main.py");
    await store.open("README.md");
    type(store, "README.md", "!");
    expect(store.dirtyPaths()).toEqual(["README.md"]);
  });
});
