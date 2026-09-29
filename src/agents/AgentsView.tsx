import { useEffect, useState } from "react";

import type { AgentChanges } from "../contracts/generated/AgentChanges";
import type { AgentSessionInfo } from "../contracts/generated/AgentSessionInfo";
import type { AgentStatus } from "../contracts/generated/AgentStatus";
import type { SessionConfiguration } from "../contracts/generated/SessionConfiguration";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import type { WorkspaceIsolation } from "../contracts/generated/WorkspaceIsolation";
import type { Store } from "../lib/store";
import { useStore } from "../lib/useStore";
import { type ModelChoice, modelChoices, type Providers } from "../models/providers";
import { type AgentRunStatus, agentRunStatus, type Terminals } from "../terminal/terminals";
import type { AgentActions } from "./actions";
import type { Agents } from "./agents";

interface Props {
  agents: Agents;
  providers: Providers;
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

/** The agents the app can run, and the sessions they work in (⇧⌘A). */
export function AgentsView({ agents, providers, terminals, workspace, actions }: Props) {
  const { agents: list, loading, environmentProblem, isolation, sessions, changes, error } = useStore(agents);
  const { providers: providerList } = useStore(providers);
  const info = useStore(workspace);
  const { panes } = useStore(terminals);

  useEffect(() => {
    if (agents.get().agents === null) void agents.load();
    // Keys and models only; local providers are looked for in the Models view.
    if (providers.get().providers === null) void providers.load();
  }, [agents, providers]);

  return (
    <div className="agents">
      <header className="explorer-header">
        <span className="explorer-title">Agents</span>
        <button type="button" className="icon-button" title="Look for installed agents again" aria-label="Refresh agents" onClick={actions.refreshAgents}>
          ↻
        </button>
      </header>
      {!info && <p className="agents-banner">Open a folder to run an agent in it.</p>}
      {info && !info.trusted && (
        <div className="agents-banner agents-banner-warning" role="status">
          <p>“{info.name}” is not trusted. Agents run only in folders you trust.</p>
          <button type="button" className="button-primary" onClick={actions.trustWorkspace}>
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
              const pane = [...panes.values()].find((p) => p.kind.type === "agent" && p.kind.session === session.id);
              return (
                <SessionCard
                  key={session.id}
                  session={session}
                  shown={shownOf(session, pane ? agentRunStatus(pane) : null)}
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
}

function IsolationNote({ isolation }: { isolation: WorkspaceIsolation }) {
  if (isolation.kind === "worktrees") {
    const from = `${isolation.branch ?? "a detached HEAD"} at ${isolation.head.slice(0, 7)}`;
    return (
      <p className="agents-note">
        Each agent works in a Git worktree of its own, starting from {from}. Your working tree is not changed; changes you have
        not committed are not included.
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
  canLaunch,
  actions,
}: {
  agent: AgentStatus;
  choices: readonly ModelChoice[];
  canLaunch: boolean;
  actions: AgentActions;
}) {
  const [chosen, setChosen] = useState(-1);
  const shown: Shown = agent.availability.state === "installed" ? "installed" : agent.availability.state;
  const installed = agent.availability.state === "installed";
  const choice = choices[chosen];
  const supported = agent.providers.filter((p) => p.supported).length;
  return (
    <li className="agent">
      <div className="agent-heading">
        <span className="agent-name">{agent.name}</span>
        <span className={`agent-status agent-status-${shown}`}>{LABELS[shown]}</span>
      </div>
      {agent.description && <p className="agent-description">{agent.description}</p>}
      <AgentDetails agent={agent} />
      {canLaunch && agent.availability.state === "installed" && (
        <p className="agent-approval">
          {agent.approved ? (
            <>
              Allowed in this folder ·{" "}
              <button type="button" className="link-button" onClick={() => actions.revokeAgent(agent.id)}>
                Revoke
              </button>
            </>
          ) : (
            "Not yet allowed in this folder"
          )}
        </p>
      )}
      {installed && supported > 0 && (
        <label className="agent-model">
          <span>Model</span>
          <select
            className="model-select"
            value={choice ? chosen : -1}
            onChange={(e) => setChosen(Number(e.target.value))}
            aria-label={`Model for ${agent.name}`}
          >
            <option value={-1}>{agent.name}'s own configuration</option>
            {choices.map((c, i) => (
              <option key={`${c.selection.provider}/${c.selection.model}`} value={i}>
                {c.provider} · {c.model}
              </option>
            ))}
          </select>
        </label>
      )}
      {installed && supported > 0 && choices.length === 0 && (
        <p className="agent-approval">To choose a model here, add a provider key or model in Models.</p>
      )}
      <button
        type="button"
        className="button-primary agent-launch"
        disabled={!canLaunch || !installed}
        onClick={() => actions.launchAgent(agent.id, choice?.selection ?? null)}
      >
        Launch
      </button>
    </li>
  );
}

function SessionCard({
  session,
  shown,
  changes,
  actions,
}: {
  session: AgentSessionInfo;
  shown: Shown;
  changes: AgentChanges | undefined;
  actions: AgentActions;
}) {
  const running = session.state.state === "running";
  const started = new Date(session.startedAt).toLocaleString([], { dateStyle: "short", timeStyle: "short" });
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
      <p className="agent-approval">
        Started {started}
        {session.state.state === "exited" && ` · exit ${session.state.exit.signal ?? session.state.exit.code}`}
      </p>
      <ConfigurationNote name={session.name} configuration={session.configuration} />
      {session.state.state === "failed" && <p className="agent-approval agents-error">{session.state.message}</p>}
      <div className="agent-actions">
        <button type="button" onClick={() => actions.openAgentTerminal(session.id)}>
          Terminal
        </button>
        <button type="button" disabled={!running} onClick={() => actions.stopAgent(session.id)}>
          Stop
        </button>
        <button type="button" onClick={() => actions.restartAgent(session.id)}>
          Restart
        </button>
        {session.worktree && (
          <button type="button" onClick={() => actions.showAgentChanges(session.id)}>
            Changes
          </button>
        )}
        <button
          type="button"
          disabled={running}
          title={running ? "Stop the agent first" : "Remove this session and its worktree"}
          onClick={() => actions.removeAgentSession(session.id)}
        >
          Remove
        </button>
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

/** Where the session's model configuration comes from; variable names only. */
function ConfigurationNote({ name, configuration }: { name: string; configuration: SessionConfiguration }) {
  if (configuration.source === "agent") {
    return (
      <p className="agent-approval" title={configuration.shellVariables.join(", ")}>
        Model: {name}'s own configuration
        {configuration.shellVariables.length > 0 && ` · from your shell: ${configuration.shellVariables.join(", ")}`}
      </p>
    );
  }
  const replaced = configuration.overriddenShellVariables;
  return (
    <>
      <p className="agent-approval" title={configuration.endpoint}>
        Model: {configuration.providerName} · {configuration.model}
        {configuration.credential === "missing" && " · no key saved"}
      </p>
      {replaced.length > 0 && (
        <p className="agent-approval" title={replaced.join(", ")}>
          Replaces from your shell: {replaced.join(", ")}
        </p>
      )}
    </>
  );
}

const CHANGE_LETTERS = { added: "A", modified: "M", deleted: "D", renamed: "R", untracked: "U", other: "·" } as const;

function AgentDetails({ agent }: { agent: AgentStatus }) {
  switch (agent.availability.state) {
    case "installed":
      return (
        <p className="agent-path" title={agent.availability.executable}>
          {agent.availability.executable}
        </p>
      );
    case "notInstalled":
      return (
        <p className="agent-path">
          <code>{agent.availability.program}</code> was not found on your PATH. Install it yourself, then refresh.
        </p>
      );
    case "unsupported":
      return <p className="agent-path">Does not run on this operating system.</p>;
  }
}

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
