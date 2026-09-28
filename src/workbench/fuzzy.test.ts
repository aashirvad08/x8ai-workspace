import { describe, expect, it } from "vitest";

import { fuzzyFilter, fuzzyScore } from "./fuzzy";

describe("fuzzy matching", () => {
  it("matches subsequences, ignoring case", () => {
    expect(fuzzyScore("mnpy", "src/main.py")).not.toBeNull();
    expect(fuzzyScore("MAIN", "src/main.py")).not.toBeNull();
    expect(fuzzyScore("xyz", "src/main.py")).toBeNull();
    expect(fuzzyScore("", "anything")).toBe(0);
  });

  it("ranks file-name and consecutive matches first", () => {
    const files = ["src/models/main_utils.py", "src/main.py", "docs/maintenance.md"];
    expect(fuzzyFilter("main.py", files, (f) => f)[0]).toBe("src/main.py");
    expect(fuzzyFilter("train", ["src/tests/rain.txt", "src/train.py"], (f) => f)[0]).toBe("src/train.py");
  });

  it("limits the results", () => {
    const many = Array.from({ length: 100 }, (_, i) => `file${i}.ts`);
    expect(fuzzyFilter("file", many, (f) => f, 10)).toHaveLength(10);
  });
});
