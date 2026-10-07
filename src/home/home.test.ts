import { describe, expect, it } from "vitest";

import { matchSpaces, shareName } from "../addons/addons";
import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import type { SpaceInfo } from "../contracts/generated/SpaceInfo";
import { Home, matchRecent, parseCommand, suggestionsFor } from "./home";

const recent: RecentWorkspace[] = [
  { id: null, root: "/Users/me/gymRL", name: "gymRL", available: true },
  { id: null, root: "/Users/me/work/x8ai-workspace", name: "x8ai-workspace", available: true },
  { id: null, root: "/Users/me/old/gymRL", name: "gymRL", available: false },
  { id: null, root: "/Users/me/labs/app", name: "app", available: true },
  { id: null, root: "/Users/me/web/app", name: "app", available: true },
];

describe("welcome commands", () => {
  it("parses commands and their argument", () => {
    expect(parseCommand("   ")).toEqual({ kind: "empty" });
    expect(parseCommand("/home")).toEqual({ kind: "command", name: "/home", arg: "" });
    expect(parseCommand("  /cd   ~/My Projects/app  ")).toEqual({ kind: "command", name: "/cd", arg: "~/My Projects/app" });
    expect(parseCommand("/name Ada Lovelace")).toEqual({ kind: "command", name: "/name", arg: "Ada Lovelace" });
    expect(parseCommand("/get codex")).toEqual({ kind: "command", name: "/get", arg: "codex" });
    expect(parseCommand("/give")).toEqual({ kind: "command", name: "/give", arg: "" });
    expect(parseCommand("cd ~/x")).toEqual({ kind: "unknown", word: "cd" });
    expect(parseCommand("/cdx")).toEqual({ kind: "unknown", word: "/cdx" });
  });

  it("suggests commands, then recent spaces for /cd", () => {
    expect(suggestionsFor("", recent)).toEqual([]);
    expect(suggestionsFor("/", recent).map((s) => s.completion)).toEqual(["/cd ", "/new ", "/share ", "/home", "/name ", "/get", "/give"]);
    expect(suggestionsFor("/g", recent).map((s) => s.label)).toEqual(["/get [agent]", "/give [agent]"]);
    expect(suggestionsFor("/h", recent).map((s) => s.label)).toEqual(["/home"]);
    expect(suggestionsFor("/cd ", recent).map((s) => s.detail)).toEqual([
      "/Users/me/gymRL",
      "/Users/me/work/x8ai-workspace",
      "/Users/me/labs/app",
      "/Users/me/web/app",
    ]);
    expect(suggestionsFor("/cd ~/gym", recent).map((s) => s.completion)).toEqual(["/cd /Users/me/gymRL"]);
    expect(suggestionsFor("/name A", recent)).toEqual([]);
  });

  it("matches a folder by its path, its end or its name, among available spaces", () => {
    expect(matchRecent("gymRL", recent).map((r) => r.root)).toEqual(["/Users/me/gymRL"]);
    expect(matchRecent("~/gymRL/", recent).map((r) => r.root)).toEqual(["/Users/me/gymRL"]);
    expect(matchRecent("/Users/me/work/x8ai-workspace", recent)).toHaveLength(1);
    expect(matchRecent("app", recent)).toHaveLength(2);
    expect(matchRecent("web/app", recent).map((r) => r.root)).toEqual(["/Users/me/web/app"]);
    expect(matchRecent("/Users/me/work", recent)).toEqual([]);
    expect(matchRecent("~", recent)).toEqual([]);
    expect(matchRecent("nothing", recent)).toEqual([]);
    // A name typed in part, when only one space starts with it.
    expect(matchRecent("gym", recent).map((r) => r.root)).toEqual(["/Users/me/gymRL"]);
    expect(matchRecent("X8AI", recent).map((r) => r.root)).toEqual(["/Users/me/work/x8ai-workspace"]);
    expect(matchRecent("a", recent)).toHaveLength(2);
    // A partial path is a path, never a name.
    expect(matchRecent("work/x8", recent)).toEqual([]);
  });
});

describe("Home", () => {
  it("greets the account's name unless another is chosen", async () => {
    const home = new Home({ getAppInfo: async () => ({ name: "x8ai", version: "0", os: "macos", arch: "aarch64", userName: "Ada Lovelace" }) });
    expect(home.get().visible).toBe(true);
    await home.load();
    expect(home.name()).toBe("Ada Lovelace");
    home.setName("  Countess  ");
    expect(home.name()).toBe("Countess");
    home.setName("x".repeat(100));
    expect(home.name()).toHaveLength(40);
    home.setName(null);
    expect(home.name()).toBe("Ada Lovelace");
    home.say("hi");
    home.hide();
    expect(home.get()).toMatchObject({ visible: false, message: null });
  });
});

const spaces: SpaceInfo[] = [
  { id: "ws-homeaa", name: "Home", root: null, addons: [] },
  { id: "ws-gymaaa", name: "gymRL", root: "/Users/me/gymRL", addons: ["starship"] },
  { id: "ws-labapp", name: "app", root: "/Users/me/labs/app", addons: [] },
  { id: "ws-webapp", name: "app", root: "/Users/me/web/app", addons: [] },
];

describe("/share", () => {
  it("names another space by id, folder, name or the start of its name, never the open one", () => {
    expect(matchSpaces("ws-gymaaa", spaces, "ws-homeaa").map((s) => s.id)).toEqual(["ws-gymaaa"]);
    expect(matchSpaces("GYM", spaces, "ws-homeaa").map((s) => s.id)).toEqual(["ws-gymaaa"]);
    expect(matchSpaces("/Users/me/web/app", spaces, null).map((s) => s.id)).toEqual(["ws-webapp"]);
    expect(matchSpaces("app", spaces, null).map((s) => s.id)).toEqual(["ws-labapp", "ws-webapp"]);
    expect(matchSpaces("gymRL", spaces, "ws-gymaaa")).toEqual([]);
    expect(matchSpaces("  ", spaces, null)).toEqual([]);
  });

  it("suggests the other spaces, by name unless two share it", () => {
    expect(suggestionsFor("/share ", recent, spaces, "ws-gymaaa").map((s) => s.completion)).toEqual([
      "/share Home",
      "/share ws-labapp",
      "/share ws-webapp",
    ]);
    expect(suggestionsFor("/share gy", recent, spaces, "ws-homeaa")).toEqual([
      { label: "gymRL", detail: "ws-gymaaa · /Users/me/gymRL", completion: "/share gymRL" },
    ]);
    expect(shareName(spaces[1]!, spaces)).toBe("gymRL");
  });
});
