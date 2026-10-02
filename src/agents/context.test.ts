import { describe, expect, it } from "vitest";

import type { AgentChanges } from "../contracts/generated/AgentChanges";
import type { AgentSessionInfo } from "../contracts/generated/AgentSessionInfo";
import {
  composeContext,
  type ContextSource,
  diffStat,
  MAX_CONTEXT_CHARS,
  MAX_DIFF_CHARS,
  matchSessions,
  pasteable,
  sessionLabel,
  stripAnsi,
  tidyOutput,
} from "./context";

const diff = [
  "diff --git a/src/main.rs b/src/main.rs",
  "--- a/src/main.rs",
  "+++ b/src/main.rs",
  "@@ -1,2 +1,3 @@",
  " fn main() {",
  "-    old();",
  "+    new();",
  "+    more();",
  "diff --git a/README.md b/README.md",
  "+++ b/README.md",
  "+# hi",
].join("\n");

const changes: AgentChanges = {
  branch: "agent/claude-code/20261002-101500-abcdef",
  base: "a".repeat(40),
  head: "b".repeat(40),
  commits: 2,
  uncommitted: true,
  files: [
    { path: "src/main.rs", change: "modified", from: null },
    { path: "README.md", change: "added", from: null },
  ],
  diff,
  truncated: false,
};

function source(change: Partial<ContextSource> = {}): ContextSource {
  return {
    name: "Claude Code",
    where: "agent/claude-code/20261002-101500-abcdef",
    changes,
    includeChanges: true,
    includeDiff: false,
    output: null,
    ...change,
  };
}

function session(id: number, name: string, branch: string | null): AgentSessionInfo {
  return {
    id,
    agent: name.toLowerCase().replace(" ", "-"),
    name,
    workspace: "/p",
    cwd: "/p",
    worktree: branch ? { branch, base: "a".repeat(40), path: `/w/${id}` } : null,
    startedAt: 0,
    state: { state: "notRunning" },
    terminal: null,
    configuration: { source: "agent", shellVariables: [] },
    mcp: [],
    skills: [],
  };
}

describe("terminal text", () => {
  it("loses escape sequences and control characters, keeps lines and tabs", () => {
    expect(stripAnsi("\x1b[1;32mok\x1b[0m\tdone\r\n\x1b]0;title\x07next\x07\x1b[?2004h")).toBe("ok\tdone\nnext");
    expect(stripAnsi("a\rb")).toBe("a\nb");
  });

  it("is tidied: trailing spaces, folded blank lines, the last lines only", () => {
    expect(tidyOutput("one   \n\n\n\ntwo\n\n")).toBe("one\n\ntwo");
    const many = Array.from({ length: 300 }, (_, i) => `line ${i}`).join("\n");
    const kept = tidyOutput(many, 150).split("\n");
    expect(kept).toHaveLength(150);
    expect(kept.at(-1)).toBe("line 299");
  });

  it("can be pasted without ending a bracketed paste early or pressing Enter", () => {
    const sneaky = "look\x1b[201~rm -rf ~\n\x1b[200~\n\n";
    const safe = pasteable(sneaky);
    expect(safe).not.toContain("\x1b");
    expect(safe.endsWith("\n")).toBe(false);
    expect(pasteable("a\r\nb\n")).toBe("a\nb");
  });
});

describe("the context text", () => {
  it("counts lines per file from the diff", () => {
    const stat = diffStat(diff);
    expect(stat.get("src/main.rs")).toEqual({ added: 2, removed: 1 });
    expect(stat.get("README.md")).toEqual({ added: 1, removed: 0 });
  });

  it("says what changed, with the diff only when chosen, and the note", () => {
    const text = composeContext([source()], "Finish the tests.");
    expect(text).toContain("## Claude Code (agent/claude-code/20261002-101500-abcdef)");
    expect(text).toContain("2 files, 2 commits, not all committed");
    expect(text).toContain("- M src/main.rs (+2 −1)");
    expect(text).toContain("- A README.md (+1 −0)");
    expect(text).not.toContain("+    more();");
    expect(text).toContain("## Note from the user\nFinish the tests.");
    expect(composeContext([source({ includeDiff: true })], "")).toContain("+    more();");
  });

  it("cuts a long diff and says so, and caps the whole text", () => {
    const big = { ...changes, diff: `diff --git a/x b/x\n${"+x\n".repeat(MAX_DIFF_CHARS)}` };
    const text = composeContext([source({ changes: big, includeChanges: false, includeDiff: true })], "");
    expect(text).toContain("The diff (truncated):");
    expect(text.length).toBeLessThan(MAX_DIFF_CHARS + 2_000);
    const huge = composeContext([], "n".repeat(MAX_CONTEXT_CHARS * 2));
    expect(huge.length).toBeLessThan(MAX_CONTEXT_CHARS + 100);
    expect(huge).toContain("(cut: too long to send at once)");
  });

  it("includes the terminal's last lines, fenced so its backticks cannot close the block", () => {
    const text = composeContext([source({ includeChanges: false, output: "\x1b[32m$ cargo test\x1b[0m\n```\nok" })], "");
    expect(text).toContain("Its terminal, last 150 lines at most:\n````\n$ cargo test\n```\nok\n````");
  });

  it("says when changes could not be read, and leaves out sources with nothing chosen", () => {
    const text = composeContext([source({ changes: null, changesProblem: "not a Git repository" })], "");
    expect(text).toContain("What changed: unknown (not a Git repository).");
    const nothing = composeContext([source({ includeChanges: false })], "");
    expect(nothing).not.toContain("## Claude Code");
  });
});

describe("sessions", () => {
  const sessions = [
    session(1, "Claude Code", "agent/claude-code/20261002-101500-abcdef"),
    session(2, "Codex", "agent/codex/20261002-111500-123456"),
    session(3, "Codex", null),
  ];

  it("are told apart by their branch", () => {
    expect(sessionLabel(sessions[1]!)).toBe("Codex · 20261002-111500-123456");
    expect(sessionLabel(sessions[2]!)).toBe("Codex · your folder");
  });

  it("are found by agent name, id or part of the branch", () => {
    expect(matchSessions("codex", sessions).map((s) => s.id)).toEqual([2, 3]);
    expect(matchSessions("claude", sessions).map((s) => s.id)).toEqual([1]);
    expect(matchSessions("111500", sessions).map((s) => s.id)).toEqual([2]);
    expect(matchSessions("  ", sessions)).toEqual([]);
  });
});
