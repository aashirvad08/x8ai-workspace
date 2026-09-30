import { describe, expect, it } from "vitest";

import { canPixelate, GLYPH_HEIGHT, pixelate } from "./pixelFont";

describe("pixel font", () => {
  it("has every capital and digit, each seven rows of one width", () => {
    for (const char of "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789,.'-! ") {
      const run = pixelate(char);
      expect(run, char).not.toBeNull();
      const rows = new Set(run!.pixels.map(([, y]) => y));
      expect([...rows].every((y) => y >= 0 && y < GLYPH_HEIGHT), char).toBe(true);
    }
    expect(pixelate("A")!.width).toBe(5);
    expect(pixelate("I")!.width).toBe(3);
  });

  it("lays out text left to right with a gap, upper-cased", () => {
    const one = pixelate("A")!;
    const two = pixelate("aA")!;
    expect(two.width).toBe(one.width * 2 + 1);
    expect(two.pixels).toHaveLength(one.pixels.length * 2);
    expect(pixelate("Welcome, Sir")).not.toBeNull();
    // Offset, for a second run after the first.
    expect(pixelate("A", 10)!.pixels.every(([x]) => x >= 10)).toBe(true);
  });

  it("gives up on characters it has no glyph for", () => {
    expect(pixelate("José")).toBeNull();
    expect(canPixelate("Ada Lovelace")).toBe(true);
    expect(canPixelate("名前")).toBe(false);
  });
});
