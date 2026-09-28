import { describe, expect, it } from "vitest";

import { layoutPanes, MIN_RATIO, pane, paneKeys, removePane, resizeSplit, splitPane } from "./panes";

describe("pane trees", () => {
  it("split a pane in place, the new one right of or below it", () => {
    const tree = splitPane(splitPane(pane(1), 1, 2, "right", 10), 2, 3, "down", 11);
    expect(paneKeys(tree)).toEqual([1, 2, 3]);
    const { panes, dividers } = layoutPanes(tree);
    expect(panes).toEqual([
      { key: 1, rect: { x: 0, y: 0, width: 0.5, height: 1 } },
      { key: 2, rect: { x: 0.5, y: 0, width: 0.5, height: 0.5 } },
      { key: 3, rect: { x: 0.5, y: 0.5, width: 0.5, height: 0.5 } },
    ]);
    expect(dividers.map((d) => [d.id, d.direction])).toEqual([
      [10, "right"],
      [11, "down"],
    ]);
  });

  it("give a closed pane's space to its sibling", () => {
    const tree = splitPane(splitPane(pane(1), 1, 2, "right", 10), 1, 3, "down", 11);
    const without = removePane(tree, 3)!;
    expect(without).toEqual(splitPane(pane(1), 1, 2, "right", 10));
    expect(removePane(removePane(without, 1)!, 2)).toBeNull();
  });

  it("leave a tree unchanged when the pane is not in it", () => {
    const tree = splitPane(pane(1), 1, 2, "right", 10);
    expect(splitPane(tree, 9, 3, "down", 11)).toBe(tree);
    expect(removePane(tree, 9)).toBe(tree);
  });

  it("keep every pane at least a minimum share when resized", () => {
    const tree = splitPane(pane(1), 1, 2, "right", 10);
    expect(layoutPanes(resizeSplit(tree, 10, 0.7)).dividers[0]!.ratio).toBe(0.7);
    expect(layoutPanes(resizeSplit(tree, 10, 0)).dividers[0]!.ratio).toBe(MIN_RATIO);
    expect(layoutPanes(resizeSplit(tree, 10, 5)).dividers[0]!.ratio).toBe(1 - MIN_RATIO);
    expect(layoutPanes(resizeSplit(tree, 10, Number.NaN)).dividers[0]!.ratio).toBe(0.5);
  });
});
