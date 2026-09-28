import { describe, expect, it } from "vitest";

import { matches, shortcutLabel } from "./commands";

const key = (key: string, mods: Partial<Record<"metaKey" | "shiftKey" | "ctrlKey" | "altKey", boolean>> = {}, code = "") => ({
  key,
  code,
  metaKey: false,
  shiftKey: false,
  ctrlKey: false,
  altKey: false,
  ...mods,
});

describe("shortcuts", () => {
  it("match letters with exactly the given modifiers", () => {
    expect(matches({ key: "s", meta: true }, key("s", { metaKey: true }))).toBe(true);
    expect(matches({ key: "s", meta: true }, key("s", { metaKey: true, shiftKey: true }))).toBe(false);
    expect(matches({ key: "p", meta: true, shift: true }, key("P", { metaKey: true, shiftKey: true }))).toBe(true);
    expect(matches({ key: "s", meta: true }, key("s"))).toBe(false);
  });

  it("match the backquote key by position", () => {
    expect(matches({ key: "`", ctrl: true }, key("`", { ctrlKey: true }, "Backquote"))).toBe(true);
    expect(matches({ key: "`", ctrl: true }, key("~", { ctrlKey: true }, "IntlBackslash"))).toBe(false);
  });

  it("render in macOS notation", () => {
    expect(shortcutLabel({ key: "p", meta: true, shift: true })).toBe("⇧⌘P");
    expect(shortcutLabel({ key: "`", ctrl: true })).toBe("⌃`");
  });
});
