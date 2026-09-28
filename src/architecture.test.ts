// Architecture fitness tests: module boundaries from docs/architecture.md, checked
// against the actual source so they cannot silently erode.
import { describe, expect, it } from "vitest";

const sources = import.meta.glob<string>(["./**/*.{ts,tsx}", "!./**/*.test.{ts,tsx}"], {
  query: "?raw",
  import: "default",
  eager: true,
});

const IMPORT = /(?:^|\s)(?:import|export)\s[^;]*?from\s*["']([^"']+)["']|import\s*\(\s*["']([^"']+)["']\s*\)|^\s*import\s*["']([^"']+)["']/gm;

function importsOf(source: string): string[] {
  return [...source.matchAll(IMPORT)].map((m) => m[1] ?? m[2] ?? m[3] ?? "");
}

function modulesWhere(predicate: (path: string, imports: string[]) => boolean): string[] {
  return Object.entries(sources)
    .filter(([path, source]) => predicate(path, importsOf(source)))
    .map(([path]) => path)
    .sort();
}

describe("module boundaries", () => {
  it("sees the source tree", () => {
    expect(Object.keys(sources)).toContain("./native/index.ts");
    expect(importsOf(sources["./native/index.ts"] ?? "")).toContain("@tauri-apps/api/core");
  });

  it("only src/native/ talks to Tauri", () => {
    const offenders = modulesWhere(
      (path, imports) => !path.startsWith("./native/") && imports.some((i) => i.startsWith("@tauri-apps/")),
    );
    expect(offenders).toEqual([]);
  });

  it("non-UI modules never depend on the UI layer", () => {
    const offenders = modulesWhere(
      (path, imports) =>
        !path.startsWith("./app/") && path !== "./main.tsx" && imports.some((i) => /(^|\/)app\//.test(i)),
    );
    expect(offenders).toEqual([]);
  });
});
