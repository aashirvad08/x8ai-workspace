import type { SkillStatus } from "../contracts/generated/SkillStatus";
import { Store } from "../lib/store";
import type { SkillApi } from "../native";

export interface SkillsSnapshot {
  readonly skills: readonly SkillStatus[] | null;
  readonly error: string | null;
}

/** Skills as the skill registry reports them: built-in and the user's. */
export class Skills extends Store<SkillsSnapshot> {
  readonly #native: Pick<SkillApi, "listSkills">;
  #generation = 0;

  constructor(native: Pick<SkillApi, "listSkills">) {
    super({ skills: null, error: null });
    this.#native = native;
  }

  async load(): Promise<void> {
    const generation = ++this.#generation;
    try {
      const list = await this.#native.listSkills();
      if (generation !== this.#generation) return;
      this.set({ skills: list.skills, error: null });
    } catch (error) {
      if (generation !== this.#generation) return;
      this.update((s) => ({ ...s, error: error instanceof Error ? error.message : String(error) }));
    }
  }

  find(id: string): SkillStatus | undefined {
    return this.get().skills?.find((s) => s.skill.id === id);
  }
}

/** The skills a new session of `agent` in `root` gets, and the per-session ones to choose. */
export function skillChoices(
  agent: { id: string; skills: { supported: boolean } },
  skills: readonly SkillStatus[],
  root: string | null,
): { always: readonly SkillStatus[]; optional: readonly SkillStatus[] } {
  if (!agent.skills.supported) return { always: [], optional: [] };
  const usable = skills.filter((s) => s.agents.some((a) => a.agent === agent.id && a.supported));
  return {
    always: usable.filter(
      (s) => s.skill.scope.kind === "global" || (s.skill.scope.kind === "workspace" && s.skill.scope.root === root),
    ),
    optional: usable.filter((s) => s.skill.scope.kind === "session"),
  };
}
