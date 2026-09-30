// Architecture fitness tests: module boundaries from docs/architecture.md, checked
// against the actual source so they cannot silently erode.
import { describe, expect, it } from "vitest";

const sources = import.meta.glob<string>(["./**/*.{ts,tsx}", "!./**/*.test.{ts,tsx}"], {
  query: "?raw",
  import: "default",
  eager: true,
});

const capabilities = import.meta.glob<string>("../src-tauri/capabilities/*.json", {
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

  it("only src/terminal/ uses the terminal emulator", () => {
    const offenders = modulesWhere(
      (path, imports) => !path.startsWith("./terminal/") && imports.some((i) => i.startsWith("@xterm/")),
    );
    expect(offenders).toEqual([]);
  });

  it("only src/editor/ uses the code editor", () => {
    const offenders = modulesWhere(
      (path, imports) =>
        !path.startsWith("./editor/") && imports.some((i) => i.startsWith("@codemirror/") || i.startsWith("@lezer/")),
    );
    expect(offenders).toEqual([]);
  });

  it("stores and the workbench stay free of React", () => {
    const offenders = modulesWhere(
      (path, imports) => !path.endsWith(".tsx") && !/\/use[A-Z]\w*\.ts$/.test(path) && imports.includes("react"),
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

  it("passes actions to event handlers bound, never as bare methods", () => {
    // `onClick={actions.refresh}` calls the method without its object, so it
    // throws and the button does nothing.
    const offenders = modulesWhere((path) => path.endsWith(".tsx") && /=\{actions\.[A-Za-z]+\}/.test(sources[path] ?? ""));
    expect(offenders).toEqual([]);
  });

  it("grants the window exactly the commands the native client calls", () => {
    // A command the client calls but the capability does not grant fails at
    // runtime; a grant nothing uses is surface for no reason.
    const client = sources["./native/client.ts"] ?? "";
    const called = [...client.matchAll(/call<[^>]*>\(\s*"([a-z_]+)"/g)].map((m) => m[1]!).sort();
    const file = capabilities["../src-tauri/capabilities/main-window.json"];
    expect(file, "the main window's capability file").toBeDefined();
    const { permissions } = JSON.parse(file!) as { permissions: string[] };
    const granted = permissions.map((p) => p.replace(/^allow-/, "").replaceAll("-", "_")).sort();
    expect(called.length).toBeGreaterThan(10);
    expect(granted).toEqual(called);
  });
});
