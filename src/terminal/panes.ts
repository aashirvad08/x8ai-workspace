/**
 * The layout of one terminal tab: a binary tree whose leaves are panes. Each
 * split divides its area between two subtrees, side by side (`right`) or one
 * above the other (`down`). Pure functions over immutable trees.
 */
export type PaneTree =
  | { readonly kind: "pane"; readonly key: number }
  | {
      readonly kind: "split";
      readonly id: number;
      /** Where the second subtree goes: to the `right` of the first, or `down` below it. */
      readonly direction: SplitDirection;
      /** The first subtree's share of the area, between MIN_RATIO and 1 - MIN_RATIO. */
      readonly ratio: number;
      readonly first: PaneTree;
      readonly second: PaneTree;
    };

export type SplitDirection = "right" | "down";

/** A rectangle as fractions of the tab's area. */
export interface Rect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface PaneLayout {
  readonly panes: readonly { readonly key: number; readonly rect: Rect }[];
  /** One per split: the line between its halves, and the area it divides. */
  readonly dividers: readonly {
    readonly id: number;
    readonly direction: SplitDirection;
    readonly ratio: number;
    readonly area: Rect;
  }[];
}

/** No pane gets less than this share of its split. */
export const MIN_RATIO = 0.1;

export function pane(key: number): PaneTree {
  return { kind: "pane", key };
}

/** Pane keys in reading order: left to right, top to bottom. */
export function paneKeys(tree: PaneTree): number[] {
  return tree.kind === "pane" ? [tree.key] : [...paneKeys(tree.first), ...paneKeys(tree.second)];
}

/** Splits pane `target` in two; the new pane `added` goes right of or below it. */
export function splitPane(tree: PaneTree, target: number, added: number, direction: SplitDirection, id: number): PaneTree {
  if (tree.kind === "pane") {
    return tree.key === target ? { kind: "split", id, direction, ratio: 0.5, first: tree, second: pane(added) } : tree;
  }
  const first = splitPane(tree.first, target, added, direction, id);
  const second = first === tree.first ? splitPane(tree.second, target, added, direction, id) : tree.second;
  return first === tree.first && second === tree.second ? tree : { ...tree, first, second };
}

/** Removes a pane; its sibling takes the space. `null` if it was the only pane. */
export function removePane(tree: PaneTree, key: number): PaneTree | null {
  if (tree.kind === "pane") return tree.key === key ? null : tree;
  const first = removePane(tree.first, key);
  if (first === null) return tree.second;
  const second = removePane(tree.second, key);
  if (second === null) return tree.first;
  return first === tree.first && second === tree.second ? tree : { ...tree, first, second };
}

export function resizeSplit(tree: PaneTree, id: number, ratio: number): PaneTree {
  if (tree.kind === "pane") return tree;
  if (tree.id === id) {
    const clamped = Math.min(1 - MIN_RATIO, Math.max(MIN_RATIO, Number.isFinite(ratio) ? ratio : 0.5));
    return clamped === tree.ratio ? tree : { ...tree, ratio: clamped };
  }
  const first = resizeSplit(tree.first, id, ratio);
  const second = resizeSplit(tree.second, id, ratio);
  return first === tree.first && second === tree.second ? tree : { ...tree, first, second };
}

/** Where each pane and divider goes. */
export function layoutPanes(tree: PaneTree): PaneLayout {
  const panes: { key: number; rect: Rect }[] = [];
  const dividers: { id: number; direction: SplitDirection; ratio: number; area: Rect }[] = [];
  const place = (node: PaneTree, area: Rect) => {
    if (node.kind === "pane") {
      panes.push({ key: node.key, rect: area });
      return;
    }
    dividers.push({ id: node.id, direction: node.direction, ratio: node.ratio, area });
    const [first, second] = divide(area, node.direction, node.ratio);
    place(node.first, first);
    place(node.second, second);
  };
  place(tree, { x: 0, y: 0, width: 1, height: 1 });
  return { panes, dividers };
}

function divide(area: Rect, direction: SplitDirection, ratio: number): [Rect, Rect] {
  if (direction === "right") {
    const width = area.width * ratio;
    return [
      { ...area, width },
      { ...area, x: area.x + width, width: area.width - width },
    ];
  }
  const height = area.height * ratio;
  return [
    { ...area, height },
    { ...area, y: area.y + height, height: area.height - height },
  ];
}
