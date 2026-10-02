import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { Layout, SAVE_DELAY_MS } from "./layout";

describe("Layout", () => {
  const written: string[] = [];
  beforeEach(() => {
    vi.useFakeTimers();
    written.length = 0;
    vi.stubGlobal("localStorage", {
      getItem: () => null,
      setItem: (_: string, value: string) => void written.push(value),
    });
  });
  afterEach(() => {
    vi.useRealTimers();
    vi.unstubAllGlobals();
  });

  it("resizes at once and writes the layout once a drag settles", () => {
    const layout = new Layout();
    for (let width = 200; width < 300; width += 5) layout.resizeExplorer(width);
    expect(layout.get().explorerWidth).toBe(295);
    expect(written).toEqual([]);
    vi.advanceTimersByTime(SAVE_DELAY_MS);
    expect(written).toHaveLength(1);
    expect(JSON.parse(written[0]!)).toMatchObject({ explorerWidth: 295 });
  });

  it("writes what is pending when asked, and nothing twice", () => {
    const layout = new Layout();
    layout.setTerminalVisible(false);
    layout.save();
    expect(JSON.parse(written[0]!)).toMatchObject({ terminalVisible: false });
    vi.advanceTimersByTime(SAVE_DELAY_MS);
    layout.save();
    expect(written).toHaveLength(1);
  });
});
