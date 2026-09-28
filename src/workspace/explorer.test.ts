import { describe, expect, it } from "vitest";

import type { DirEntry } from "../contracts/generated/DirEntry";
import { NativeError } from "../native";
import { Explorer, visibleRows } from "./explorer";

const file = (path: string): DirEntry => ({ name: path.split("/").pop()!, path, kind: "file", symlink: false });
const dir = (path: string): DirEntry => ({ name: path.split("/").pop()!, path, kind: "directory", symlink: false });

/** A fake tree; `reads` records which directories were listed. */
function setup(tree: Record<string, DirEntry[]>) {
  const reads: string[] = [];
  const explorer = new Explorer({
    listDir: async (path: string) => {
      reads.push(path);
      const entries = tree[path];
      if (!entries) throw new NativeError("workspace_list_dir", "notFound", `"${path}" does not exist`);
      return entries;
    },
  });
  return { explorer, reads, tree };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

const labels = (explorer: Explorer) =>
  visibleRows(explorer.get()).map((row) =>
    row.type === "entry" ? `${"  ".repeat(row.depth)}${row.entry.name}${row.expanded ? "/" : ""}` : `<${row.type}>`,
  );

describe("Explorer", () => {
  const tree = {
    "": [dir("src"), dir("tests"), file("README.md")],
    src: [file("src/main.py"), file("src/train.py")],
    tests: [file("tests/test_main.py")],
  };

  it("lists the root when a workspace opens, and nothing else", async () => {
    const { explorer, reads } = setup(tree);
    explorer.reset(true);
    await settle();
    expect(labels(explorer)).toEqual(["src", "tests", "README.md"]);
    expect(reads).toEqual([""]);
  });

  it("loads a directory the first time it is expanded", async () => {
    const { explorer, reads } = setup(tree);
    explorer.reset(true);
    await settle();
    explorer.toggle("src");
    expect(labels(explorer)).toContain("<status>");
    await settle();
    expect(labels(explorer)).toEqual(["src/", "  main.py", "  train.py", "tests", "README.md"]);

    explorer.toggle("src");
    explorer.toggle("src");
    await settle();
    expect(reads).toEqual(["", "src"]);
  });

  it("reloads only loaded directories touched by a change", async () => {
    const { explorer, reads, tree: t } = setup({ ...tree });
    explorer.reset(true);
    await settle();
    explorer.expand("src");
    await settle();
    reads.length = 0;

    t.src = [...(t.src ?? []), file("src/new.py")];
    await explorer.diskChanged(["src/new.py", "tests/test_main.py"]);
    expect(reads).toEqual(["src"]);
    expect(labels(explorer)).toContain("  new.py");
  });

  it("forgets a directory that disappeared", async () => {
    const { explorer, tree: t } = setup({ ...tree });
    explorer.reset(true);
    await settle();
    explorer.expand("src");
    await settle();

    delete t.src;
    t[""] = [dir("tests"), file("README.md")];
    await explorer.diskChanged(["src"]);
    expect(explorer.get().listings.has("src")).toBe(false);
    expect(explorer.get().expanded.has("src")).toBe(false);
    expect(labels(explorer)).toEqual(["tests", "README.md"]);
  });

  it("shows listing errors in the tree", async () => {
    const explorer = new Explorer({
      listDir: async () => {
        throw new NativeError("workspace_list_dir", "permissionDenied", "permission denied");
      },
    });
    explorer.reset(true);
    await settle();
    const rows = visibleRows(explorer.get());
    expect(rows).toEqual([{ type: "status", dir: "", depth: 0, text: "permission denied", error: true }]);
  });

  it("places an inline name field in the target directory", async () => {
    const { explorer } = setup(tree);
    explorer.reset(true);
    await settle();
    explorer.startEditing({ kind: "newFile", parent: "src" });
    await settle();
    expect(labels(explorer)).toEqual(["src/", "<new>", "  main.py", "  train.py", "tests", "README.md"]);
  });

  it("keeps expansion and selection across a rename", async () => {
    const { explorer, reads } = setup({ ...tree, lib: tree.src });
    explorer.reset(true);
    await settle();
    explorer.expand("src");
    explorer.select("src/main.py");
    explorer.renamed("src", "lib");
    expect(explorer.get().expanded.has("lib")).toBe(true);
    expect(explorer.get().selected).toBe("lib/main.py");
    await settle();
    expect(reads).toContain("lib");
  });
});
