import { describe, expect, it } from "vitest";

import { LaunchDrafts } from "./draft";

describe("LaunchDrafts", () => {
  it("keeps each agent's next launch apart, without duplicates", () => {
    const drafts = new LaunchDrafts();
    expect(drafts.of("claude-code")).toEqual({ model: null, mcp: [], skills: [] });
    drafts.setModel("claude-code", { provider: "anthropic", model: "claude-sonnet-5" });
    drafts.setMcp("claude-code", "github", true);
    drafts.setMcp("claude-code", "github", true);
    drafts.setSkill("claude-code", "tests-first", true);
    drafts.setSkill("opencode", "python-debugging", true);
    expect(drafts.of("claude-code")).toEqual({
      model: { provider: "anthropic", model: "claude-sonnet-5" },
      mcp: ["github"],
      skills: ["tests-first"],
    });
    expect(drafts.of("opencode").skills).toEqual(["python-debugging"]);
    drafts.setMcp("claude-code", "github", false);
    drafts.setModel("claude-code", null);
    expect(drafts.of("claude-code")).toEqual({ model: null, mcp: [], skills: ["tests-first"] });
  });
});
