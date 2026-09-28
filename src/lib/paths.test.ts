import { describe, expect, it } from "vitest";

import { basename, dirname, isWithin, join, rebase } from "./paths";

describe("workspace paths", () => {
  it("splits and joins", () => {
    expect(basename("src/app/main.ts")).toBe("main.ts");
    expect(basename("main.ts")).toBe("main.ts");
    expect(dirname("src/app/main.ts")).toBe("src/app");
    expect(dirname("main.ts")).toBe("");
    expect(join("", "a")).toBe("a");
    expect(join("src", "a")).toBe("src/a");
  });

  it("knows what lies beneath a directory", () => {
    expect(isWithin("src/a.ts", "src")).toBe(true);
    expect(isWithin("src", "src")).toBe(true);
    expect(isWithin("srcfoo/a.ts", "src")).toBe(false);
    expect(isWithin("anything", "")).toBe(true);
  });

  it("rebases renamed paths", () => {
    expect(rebase("src/a.ts", "src", "lib")).toBe("lib/a.ts");
    expect(rebase("src", "src", "lib")).toBe("lib");
  });
});
