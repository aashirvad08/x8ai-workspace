import { describe, expect, it } from "vitest";

import { Terminals } from "../terminal/terminals";
import { Value } from "./store";
import { sameItems, selection } from "./useStore";

describe("selection", () => {
  it("keeps the selected value while the part it depends on is unchanged", () => {
    const store = new Value({ count: 1, other: "a" });
    const pick = selection(store, (s) => ({ count: s.count }), (a, b) => a.count === b.count);
    const first = pick.get();
    store.set({ count: 1, other: "b" });
    expect(pick.get()).toBe(first);
    store.set({ count: 2, other: "b" });
    expect(pick.get()).not.toBe(first);
    expect(pick.get().count).toBe(2);
  });

  it("gives the agents view the same agent panes through focus changes, tab switches and split drags", () => {
    const terminals = new Terminals();
    terminals.add();
    terminals.add({ type: "agent", agent: "claude-code", name: "Claude Code", session: 1 });
    const pick = selection(terminals, () => terminals.agentPanes(), sameItems);
    const before = pick.get();
    expect(before).toHaveLength(1);

    const [shell, agent] = terminals.get().tabs;
    terminals.activate(shell!.key);
    terminals.split("right");
    terminals.focusNext(1);
    terminals.resize(shell!.key, 1, 0.3);
    terminals.requestFocus();
    expect(pick.get()).toBe(before);

    // The agent's own pane changing is a new selection.
    terminals.started(agent!.focused, { id: 7, program: "/bin/claude", cwd: "/w", ackBytes: 65536 });
    expect(pick.get()).not.toBe(before);
    expect(pick.get()[0]?.running).toBe(true);
  });
});
