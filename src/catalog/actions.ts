import type { SkillInput } from "../contracts/generated/SkillInput";

/**
 * What the catalog view can ask for. The workbench implements each by asking the
 * system that owns the item: nothing here runs, installs, approves or unlocks.
 */
export interface CatalogActions {
  refreshCatalog(): void;
  /** Shows the agent in the Agents view. */
  openAgent(agent: string): void;
  /** Chooses the model for the next launch of each agent that can use it. */
  chooseModel(provider: string, model: string): void;
  /** Opens a session with the model now, in the agent that can use it (as dropping it on the terminal does). */
  launchModel(provider: string, model: string): void;
  /** Shows the provider in the Models view. */
  configureProvider(provider: string): void;
  /** Attaches a session MCP server to the next launch of each agent that can use it. */
  attachMcp(server: string): void;
  /** Shows the MCP view. */
  configureMcp(server: string): void;
  setMcpServerEnabled(id: string, enabled: boolean): void;
  /** Attaches a session skill to the next launch of each agent that can take it. */
  attachSkill(skill: string): void;
  addSkill(skill: SkillInput): Promise<boolean>;
  updateSkill(id: string, skill: SkillInput): Promise<boolean>;
  /** Removes a user skill, after asking. */
  removeSkill(id: string): void;
}
