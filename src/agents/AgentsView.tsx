import { memo, useEffect } from "react";

import type { AgentChanges } from "../contracts/generated/AgentChanges";
import type { AgentSessionInfo } from "../contracts/generated/AgentSessionInfo";
import type { AgentStatus } from "../contracts/generated/AgentStatus";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import type { WorkspaceIsolation } from "../contracts/generated/WorkspaceIsolation";
import type { Store } from "../lib/store";
import { sameItems, useSelected, useStore } from "../lib/useStore";
import type { SessionMcpServer } from "../contracts/generated/SessionMcpServer";
import type { SessionSkill } from "../contracts/generated/SessionSkill";
import type { SkillStatus } from "../contracts/generated/SkillStatus";
import { type Skills, skillChoices } from "../skills/skills";
import type { LaunchDrafts } from "./draft";
import { type McpChoices, mcpChoices, type McpServers } from "../mcp/servers";
import { type ModelChoice, modelChoices, type Providers } from "../models/providers";
import { type AgentRunStatus, agentRunStatus, type Terminals } from "../terminal/terminals";
import type { AgentActions } from "./actions";
import type { Agents } from "./agents";

interface Props {
  agents: Agents;
  providers: Providers;
  mcp: McpServers;
  skills: Skills;
  drafts: LaunchDrafts;
  terminals: Terminals;
  workspace: Store<WorkspaceInfo | null>;
  actions: AgentActions;
}

type Shown = "notInstalled" | "unsupported" | "installed" | "stopped" | AgentRunStatus;

const LABELS: Record<Shown, string> = {
  notInstalled: "Not installed",
  unsupported: "Not available here",
  installed: "Installed",
  stopped: "Stopped",
  starting: "Starting",
  running: "Running",
  exited: "Exited",
  failed: "Failed",
};

/**
 * The agents the app can run, and the sessions they work in (⇧⌘A). Its props are
 * the workbench's stores, so it renders only when they change, not with the
 * sidebar; of the terminals it follows only the agent panes.
 */
export const AgentsView = memo(function AgentsView({ agents, providers, mcp, skills, drafts, terminals, workspace, actions }: Props) {
  const { agents: list, loading, environmentProblem, isolation, sessions, changes, error } = useStore(agents);
  const { providers: providerList } = useStore(providers);
  const { servers: mcpList } = useStore(mcp);
  const { skills: skillList } = useStore(skills);
  useStore(drafts);
  const info = useStore(workspace);
  const agentPanes = useSelected(terminals, () => terminals.agentPanes(), sameItems);

  useEffect(() => {
    if (agents.get().agents === null) void agents.load();
    // Keys and models only; local providers are looked for in the Models view.
    if (providers.get().providers === null) void providers.load();
    if (mcp.get().servers === null) void mcp.load();
    if (skills.get().skills === null) void skills.load();
  }, [agents, providers, mcp, skills]);

  return (
    <div className="agents">
      <header className="explorer-header">
        <span className="explorer-title">Agents</span>
        <button type="button" className="icon-button" title="Look for installed agents again" aria-label="Refresh agents" onClick={() => actions.refreshAgents()}>
          ↻
        </button>
      </header>
      {!info && <p className="agents-banner">Open a folder to run an agent in it.</p>}
      {info && !info.trusted && (
        <div className="agents-banner agents-banner-warning" role="status">
          <p>“{info.name}” is not trusted. Agents run only in folders you trust.</p>
          <button type="button" className="button-primary" onClick={() => actions.trustWorkspace()}>
            Trust Folder…
          </button>
        </div>
      )}
      {info && isolation && <IsolationNote isolation={isolation} />}
      {environmentProblem && (
        <p className="agents-note" role="status">
          Your shell environment could not be read, so agents were looked for on the app's own PATH: {environmentProblem}
        </p>
      )}
      {error && (
        <p className="agents-note agents-error" role="alert">
          {error}
        </p>
      )}
      {list === null && loading && <p className="agents-note">Looking for installed agents…</p>}
      <ul className="agents-list" aria-label="Agents">
        {list?.map((agent) => (
          <AgentCard
            key={agent.id}
            agent={agent}
            choices={modelChoices(agent, providerList ?? [])}
            mcpChoices={mcpChoices(agent, mcpList ?? [], info?.root ?? null)}
            skillChoices={skillChoices(agent, skillList ?? [], info?.root ?? null)}
            drafts={drafts}
            canLaunch={info !== null}
            actions={actions}
          />
        ))}
      </ul>
      {sessions.length > 0 && (
        <>
          <h2 className="recent-title agents-heading">Sessions</h2>
          <ul className="agents-list" aria-label="Agent sessions">
            {[...sessions].reverse().map((session) => {
              const pane = agentPanes.find((p) => p.kind.type === "agent" && p.kind.session === session.id);
              return (
                <SessionCard
                  key={session.id}
                  session={session}
                  shown={shownOf(session, pane ? agentRunStatus(pane) : null)}
                  open={pane !== undefined}
                  changes={changes.get(session.id)}
                  actions={actions}
                />
              );
            })}
          </ul>
        </>
      )}
    </div>
  );
});

function IsolationNote({ isolation }: { isolation: WorkspaceIsolation }) {
  if (isolation.kind === "worktrees") {
    const from = `${isolation.branch ?? "a detached HEAD"} at ${isolation.head.slice(0, 7)}`;
    return (
      <p
        className="agents-hint"
        title={`Each agent works in a Git worktree of its own, starting from ${from}. Your working tree is not changed; changes you have not committed are not included.`}
      >
        A worktree per session, from {isolation.branch ?? "HEAD"}@{isolation.head.slice(0, 7)}
      </p>
    );
  }
  return (
    <p className="agents-note agents-note-warning" role="status">
      {isolation.reason} Agents are not isolated here: one at a time runs directly in this folder.
    </p>
  );
}

function AgentCard({
  agent,
  choices,
  mcpChoices,
  skillChoices,
  drafts,
  canLaunch,
  actions,
}: {
  agent: AgentStatus;
  choices: readonly ModelChoice[];
  mcpChoices: McpChoices;
  skillChoices: { always: readonly SkillStatus[]; optional: readonly SkillStatus[] };
  drafts: LaunchDrafts;
  canLaunch: boolean;
  actions: AgentActions;
}) {
  const draft = drafts.of(agent.id);
  const chosen = choices.findIndex(
    (c) => c.selection.provider === draft.model?.provider && c.selection.model === draft.model?.model,
  );
  const choice = choices[chosen];
  const supported = agent.providers.filter((p) => p.supported).length;
  if (agent.availability.state !== "installed") {
    // Nothing to do with it here: one quiet line, the reason on hover.
    const shown: Shown = agent.availability.state;
    return (
      <li className="agent agent-unavailable" title={unavailableReason(agent)}>
        <div className="agent-heading">
          <span className="agent-name">{agent.name}</span>
          <span className="agent-status">{LABELS[shown]}</span>
        </div>
      </li>
    );
  }
  const executable = agent.availability.executable;
  return (
    <li className="agent">
      <div className="agent-heading">
        <span className="agent-name" title={[agent.description, executable].filter(Boolean).join("\n")}>
          {agent.name}
        </span>
        <button
          type="button"
          className="button-primary agent-launch"
          disabled={!canLaunch}
          title={canLaunch ? `Start ${agent.name} in a session of its own` : "Open a folder first"}
          onClick={() =>
            actions.launchAgent(
              agent.id,
              choice?.selection ?? null,
              draft.mcp.filter((id) => mcpChoices.optional.some((s) => s.server.id === id)),
              draft.skills.filter((id) => skillChoices.optional.some((s) => s.skill.id === id)),
            )
          }
        >
          Launch
        </button>
      </div>
      {supported > 0 && choices.length > 0 && (
        <label className="agent-model">
          <span>Model</span>
          <select
            className="model-select"
            value={choice ? chosen : -1}
            onChange={(e) => drafts.setModel(agent.id, choices[Number(e.target.value)]?.selection ?? null)}
            aria-label={`Model for ${agent.name}`}
          >
            <option value={-1}>Its own configuration</option>
            {choices.map((c, i) => (
              <option key={`${c.selection.provider}/${c.selection.model}`} value={i}>
                {c.provider} · {c.model}
              </option>
            ))}
          </select>
        </label>
      )}
      {supported > 0 && choices.length === 0 && <p className="agent-hint">No models yet · add a key in Models</p>}
      <LaunchOptions
        agent={agent}
        mcp={mcpChoices}
        skills={skillChoices}
        chosenMcp={draft.mcp}
        chosenSkills={draft.skills}
        onMcp={(id, on) => drafts.setMcp(agent.id, id, on)}
        onSkill={(id, on) => drafts.setSkill(agent.id, id, on)}
      />
      {canLaunch && agent.approved && (
        <p className="agent-hint">
          Allowed here ·{" "}
          <button type="button" className="link-button" onClick={() => actions.revokeAgent(agent.id)}>
            Revoke
          </button>
        </p>
      )}
    </li>
  );
}

function unavailableReason(agent: AgentStatus): string {
  switch (agent.availability.state) {
    case "notInstalled":
      return `${agent.availability.program} was not found on your PATH. Install it yourself, then refresh.`;
    case "unsupported":
      return "Does not run on this operating system.";
    case "installed":
      return "";
  }
}

/**
 * The MCP servers and skills a launch gets, folded away: the ones every session
 * gets, and checkboxes for the ones chosen per launch. Nothing for what the
 * agent cannot take.
 */
function LaunchOptions({
  agent,
  mcp,
  skills,
  chosenMcp,
  chosenSkills,
  onMcp,
  onSkill,
}: {
  agent: AgentStatus;
  mcp: McpChoices;
  skills: { always: readonly SkillStatus[]; optional: readonly SkillStatus[] };
  chosenMcp: readonly string[];
  chosenSkills: readonly string[];
  onMcp: (id: string, on: boolean) => void;
  onSkill: (id: string, on: boolean) => void;
}) {
  const mcpOn = agent.mcp.supported;
  const skillsOn = agent.skills.supported;
  const always = [
    ...(mcpOn ? mcp.always.map((s) => `MCP ${s.server.name}`) : []),
    ...(skillsOn ? skills.always.map((s) => s.skill.name) : []),
  ];
  const optionalMcp = mcpOn ? mcp.optional : [];
  const optionalSkills = skillsOn ? skills.optional : [];
  if (always.length + optionalMcp.length + optionalSkills.length === 0) return null;
  const on =
    optionalMcp.filter((s) => chosenMcp.includes(s.server.id)).length +
    optionalSkills.filter((s) => chosenSkills.includes(s.skill.id)).length;
  return (
    <details className="agent-options">
      <summary>
        Options
        {on > 0 && <span className="agent-options-count">{on} on</span>}
      </summary>
      {always.length > 0 && <p className="agents-hint">Always: {always.join(", ")}</p>}
      {optionalMcp.map((s) => (
        <label key={s.server.id} className="agent-check">
          <input
            type="checkbox"
            checked={chosenMcp.includes(s.server.id)}
            aria-label={`Attach ${s.server.name}`}
            onChange={(e) => onMcp(s.server.id, e.target.checked)}
          />
          <span>MCP · {s.server.name}</span>
        </label>
      ))}
      {optionalSkills.map((s) => (
        <label key={s.skill.id} className="agent-check" title={s.skill.description}>
          <input
            type="checkbox"
            checked={chosenSkills.includes(s.skill.id)}
            aria-label={`Attach the skill ${s.skill.name}`}
            onChange={(e) => onSkill(s.skill.id, e.target.checked)}
          />
          <span>{s.skill.name}</span>
        </label>
      ))}
    </details>
  );
}

function SessionCard({
  session,
  shown,
  open,
  changes,
  actions,
}: {
  session: AgentSessionInfo;
  shown: Shown;
  /** It has a terminal pane, which the main action shows rather than starting it again. */
  open: boolean;
  changes: AgentChanges | undefined;
  actions: AgentActions;
}) {
  const running = session.state.state === "running";
  const started = new Date(session.startedAt).toLocaleString([], { dateStyle: "short", timeStyle: "short" });
  const configuration = session.configuration;
  const model =
    configuration.source === "app"
      ? `${configuration.providerName} · ${configuration.model}${configuration.credential === "missing" ? " · no key saved" : ""}`
      : "own configuration";
  const replaced =
    configuration.source === "app" ? configuration.overriddenShellVariables : configuration.shellVariables;
  return (
    <li className="agent agent-session" aria-label={`${session.name} session`}>
      <div className="agent-heading">
        <span className={running ? "dot dot-ok" : "dot"} aria-hidden />
        <span className="agent-name">{session.name}</span>
        <span className={`agent-status agent-status-${shown}`}>{LABELS[shown]}</span>
      </div>
      <p className="agent-path" title={session.cwd}>
        {session.worktree ? session.worktree.branch : "In your folder (not isolated)"}
      </p>
      <p
        className="agent-hint"
        title={[
          `Started ${started}`,
          configuration.source === "app" ? configuration.endpoint : "",
          replaced.length > 0 ? `${configuration.source === "app" ? "Replaces" : "From your shell"}: ${replaced.join(", ")}` : "",
        ]
          .filter(Boolean)
          .join("\n")}
      >
        {model} · {ago(session.startedAt)}
        {session.state.state === "exited" && ` · exit ${session.state.exit.signal ?? session.state.exit.code}`}
      </p>
      {session.mcp.length > 0 && <SessionMcp servers={session.mcp} />}
      {session.skills.length > 0 && <SessionSkills skills={session.skills} />}
      {session.state.state === "failed" && <p className="agent-approval agents-error">{session.state.message}</p>}
      <div className="agent-actions">
        <button type="button" className="agent-action-main" onClick={() => actions.openAgentTerminal(session.id)}>
          {running || open ? "Show" : "Start"}
        </button>
        {running && (
          <button type="button" onClick={() => actions.stopAgent(session.id)}>
            Stop
          </button>
        )}
        {running && (
          <button type="button" onClick={() => actions.restartAgent(session.id)}>
            Restart
          </button>
        )}
        {session.worktree && (
          <button type="button" title="What it changed, read-only" onClick={() => actions.showAgentChanges(session.id)}>
            Changes
          </button>
        )}
        <button type="button" title="Give this agent what other sessions did" onClick={() => actions.shareContext("get", session.id)}>
          Get
        </button>
        <button type="button" title="Pass this session's work to another one" onClick={() => actions.shareContext("give", session.id)}>
          Give
        </button>
        {!running && (
          <button type="button" title="Remove this session and its worktree" onClick={() => actions.removeAgentSession(session.id)}>
            Remove
          </button>
        )}
      </div>
      {changes && (
        <div className="agent-changes">
          <p className="agent-approval">
            {changes.files.length === 0
              ? "No changes"
              : `${changes.files.length} changed file${changes.files.length === 1 ? "" : "s"}`}
            {changes.commits > 0 && ` · ${changes.commits} commit${changes.commits === 1 ? "" : "s"}`}
            {changes.uncommitted && " · not all committed"}
          </p>
          <ul className="agent-files">
            {changes.files.map((file) => (
              <li key={file.path}>
                <button
                  type="button"
                  className="agent-file"
                  disabled={file.change === "deleted"}
                  title={file.from ? `${file.from} → ${file.path}` : file.path}
                  onClick={() => actions.openAgentFile(session.id, file.path)}
                >
                  <span className={`agent-change agent-change-${file.change}`}>{CHANGE_LETTERS[file.change]}</span>
                  {file.path}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </li>
  );
}

/** "just now", "5m ago", "3h ago", "2d ago". */
export function ago(at: number, now = Date.now()): string {
  const minutes = Math.floor((now - at) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h ago`;
  return `${Math.floor(hours / 24)}d ago`;
}

const SKILL_STATES = { attached: "attached", changed: "changed", removed: "removed" } as const;

/** The session's skills, as recorded; a changed or removed one stops it running. */
function SessionSkills({ skills }: { skills: readonly SessionSkill[] }) {
  return (
    <ul className="agent-attached" aria-label="Skills">
      {skills.map((s) => (
        <li key={s.id} className={s.state.state === "attached" ? "agent-hint" : "agent-hint agents-error"}>
          Skill: {s.name} v{s.version} · {SKILL_STATES[s.state.state]}
          {s.state.state === "changed" && ` (now v${s.state.currentVersion}): start a new session to use it`}
          {s.state.state === "removed" && ": start a new session without it"}
        </li>
      ))}
    </ul>
  );
}

const MCP_STATES = {
  idle: "not running",
  remote: "remote",
  waiting: "ready",
  running: "running",
  exited: "ended",
  failed: "failed",
  skipped: "not used",
} as const;

/** The session's MCP servers and what each is doing; never a secret. */
function SessionMcp({ servers }: { servers: readonly SessionMcpServer[] }) {
  return (
    <ul className="agent-attached" aria-label="MCP servers">
      {servers.map((s) => {
        const detail = s.state.state === "failed" ? s.state.message : s.state.state === "skipped" ? s.state.reason : undefined;
        return (
          <li key={s.id} className={s.state.state === "failed" ? "agent-hint agents-error" : "agent-hint"} title={detail}>
            MCP: {s.name} · {MCP_STATES[s.state.state]}
            {s.state.state === "running" && ` (pid ${s.state.pid})`}
            {detail && ` · ${detail}`}
          </li>
        );
      })}
    </ul>
  );
}

const CHANGE_LETTERS = { added: "A", modified: "M", deleted: "D", renamed: "R", untracked: "U", other: "·" } as const;

/** The pane knows best while it exists (it sees "starting"); otherwise the native state. */
function shownOf(session: AgentSessionInfo, pane: AgentRunStatus | null): Shown {
  if (pane) return pane;
  switch (session.state.state) {
    case "running":
      return "running";
    case "exited":
      return session.state.exit.code === 0 && !session.state.exit.signal ? "exited" : "failed";
    case "failed":
      return "failed";
    case "notRunning":
      return "stopped";
  }
}
