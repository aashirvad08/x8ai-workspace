import { describe, expect, it } from "vitest";

import { paneKeys } from "./panes";
import { tabTitle, Terminals } from "./terminals";

const info = (id: number) => ({ id, program: "/bin/zsh", cwd: "/Users/me/project", ackBytes: 65536 });

describe("terminals", () => {
  it("give every pane its own session and title", () => {
    const terminals = new Terminals();
    const tab = terminals.add();
    const first = terminals.activeTab()!.focused;
    terminals.split("right");
    const second = terminals.activeTab()!.focused;
    expect(second).not.toBe(first);

    terminals.started(first, info(1));
    terminals.started(second, info(2));
    expect(terminals.liveSessions(tab)).toEqual([1, 2]);
    expect(tabTitle(terminals.activeTab()!, terminals.get().panes)).toBe("zsh — project (2)");

    terminals.ended(first);
    expect(terminals.liveSessions()).toEqual([2]);
  });

  it("move focus to a neighbour when the focused pane closes", () => {
    const terminals = new Terminals();
    terminals.add();
    const first = terminals.activeTab()!.focused;
    terminals.split("right");
    terminals.split("down");
    const third = terminals.activeTab()!.focused;
    const second = paneKeys(terminals.activeTab()!.tree)[1]!;

    terminals.closePane(third);
    expect(terminals.activeTab()!.focused).toBe(second);
    expect(terminals.get().panes.has(third)).toBe(false);

    terminals.focusNext(1);
    expect(terminals.activeTab()!.focused).toBe(first);
    terminals.focusNext(-1);
    expect(terminals.activeTab()!.focused).toBe(second);
  });

  it("close the tab with its last pane", () => {
    const terminals = new Terminals();
    const tab = terminals.add();
    terminals.add();
    terminals.activate(tab);
    terminals.closePane(terminals.activeTab()!.focused);
    expect(terminals.get().tabs.map((t) => t.key)).not.toContain(tab);
    expect(terminals.get().panes.size).toBe(1);
    expect(terminals.activeTab()).toBeDefined();
  });

  it("focus a pane in another tab by activating that tab", () => {
    const terminals = new Terminals();
    terminals.add();
    const pane = terminals.activeTab()!.focused;
    const other = terminals.add();
    expect(terminals.get().active).toBe(other);
    terminals.focusPane(pane);
    expect(terminals.activeTab()!.focused).toBe(pane);
    expect(terminals.get().active).not.toBe(other);
  });

  it("split opens a first terminal when there is none", () => {
    const terminals = new Terminals();
    terminals.split("down");
    expect(terminals.get().tabs).toHaveLength(1);
    expect(terminals.activeTab()!.tree.kind).toBe("pane");
  });
});
