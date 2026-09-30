import { describe, expect, it } from "vitest";

import type { SkillStatus } from "../contracts/generated/SkillStatus";
import { skillChoices } from "./skills";

function skill(id: string, scope: SkillStatus["skill"]["scope"], supported = true): SkillStatus {
  return {
    skill: { id, name: id, description: "", version: 1, instructions: "x", allowedTools: [], source: "user", scope },
    agents: [{ agent: "claude-code", supported, reason: supported ? null : "no" }],
  };
}

describe("skill launch choices", () => {
  const skills = [
    skill("global", { kind: "global" }),
    skill("here", { kind: "workspace", root: "/Users/me/project" }),
    skill("elsewhere", { kind: "workspace", root: "/Users/me/other" }),
    skill("optional", { kind: "session" }),
    skill("unsupported", { kind: "global" }, false),
  ];

  it("attaches global and this folder's skills and offers session skills", () => {
    const choices = skillChoices({ id: "claude-code", skills: { supported: true } }, skills, "/Users/me/project");
    expect(choices.always.map((s) => s.skill.id)).toEqual(["global", "here"]);
    expect(choices.optional.map((s) => s.skill.id)).toEqual(["optional"]);
  });

  it("offers nothing to an agent that cannot take skills", () => {
    expect(skillChoices({ id: "opencode", skills: { supported: false } }, skills, "/Users/me/project")).toEqual({ always: [], optional: [] });
    expect(skillChoices({ id: "opencode", skills: { supported: true } }, skills, null).optional).toEqual([]);
  });
});
