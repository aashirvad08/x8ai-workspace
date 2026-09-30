import type { ModelSelection } from "../contracts/generated/ModelSelection";
import { Store } from "../lib/store";

/** What the next launch of an agent will ask for. Nothing is approved or started by it. */
export interface LaunchDraft {
  readonly model: ModelSelection | null;
  /** Session-scoped MCP server ids. */
  readonly mcp: readonly string[];
  /** Session-scoped skill ids. */
  readonly skills: readonly string[];
}

const EMPTY: LaunchDraft = { model: null, mcp: [], skills: [] };

/** Launch choices per agent, set on the agent's card or from the catalog. */
export class LaunchDrafts extends Store<ReadonlyMap<string, LaunchDraft>> {
  constructor() {
    super(new Map());
  }

  of(agent: string): LaunchDraft {
    return this.get().get(agent) ?? EMPTY;
  }

  setModel(agent: string, model: ModelSelection | null): void {
    this.#change(agent, (d) => ({ ...d, model }));
  }

  setMcp(agent: string, id: string, on: boolean): void {
    this.#change(agent, (d) => ({ ...d, mcp: toggle(d.mcp, id, on) }));
  }

  setSkill(agent: string, id: string, on: boolean): void {
    this.#change(agent, (d) => ({ ...d, skills: toggle(d.skills, id, on) }));
  }

  #change(agent: string, change: (draft: LaunchDraft) => LaunchDraft): void {
    this.update((all) => new Map(all).set(agent, change(all.get(agent) ?? EMPTY)));
  }
}

function toggle(list: readonly string[], id: string, on: boolean): readonly string[] {
  const without = list.filter((x) => x !== id);
  return on ? [...without, id] : without;
}
