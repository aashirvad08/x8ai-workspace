import { useEffect } from "react";

import type { AgentStatus } from "../contracts/generated/AgentStatus";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import type { Store } from "../lib/store";
import { useStore } from "../lib/useStore";
import { agentRunStatus, type AgentRunStatus, type Terminals } from "../terminal/terminals";
import type { AgentActions } from "./actions";
import type { Agents } from "./agents";

interface Props {
  agents: Agents;
  terminals: Terminals;
  workspace: Store<WorkspaceInfo | null>;
  actions: AgentActions;
}

type Shown = "notInstalled" | "unsupported" | "installed" | AgentRunStatus;

const LABELS: Record<Shown, string> = {
  notInstalled: "Not installed",
  unsupported: "Not available here",
  installed: "Installed",
  starting: "Starting",
  running: "Running",
  exited: "Exited",
  failed: "Failed",
};

/** The agents the app can run, and what they are doing (⇧⌘A). */
export function AgentsView({ agents, terminals, workspace, actions }: Props) {
  const { agents: list, loading, environmentProblem, error } = useStore(agents);
  const info = useStore(workspace);
  const { panes } = useStore(terminals);

  useEffect(() => {
    if (agents.get().agents === null) void agents.load();
  }, [agents]);

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
          <p>
            “{info.name}” is not trusted. Agents run only in folders you trust.
          </p>
          <button type="button" className="button-primary" onClick={actions.trustWorkspace}>
            Trust Folder…
          </button>
        </div>
      )}
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
      <ul className="agents-list">
        {list?.map((agent) => {
          const agentPanes = [...panes.values()].filter((p) => p.kind.type === "agent" && p.kind.agent === agent.id);
          // The most recently opened pane for this agent says what it is doing.
          const latest = agentPanes.at(-1);
          const shown: Shown = latest ? agentRunStatus(latest) : availabilityOf(agent);
          const running = agentPanes.filter((p) => p.running).length;
          return (
            <li key={agent.id} className="agent">
              <div className="agent-heading">
                <span className="agent-name">{agent.name}</span>
                <span className={`agent-status agent-status-${shown}`}>{LABELS[shown]}</span>
              </div>
              {agent.description && <p className="agent-description">{agent.description}</p>}
              <AgentDetails agent={agent} />
              {info && agent.availability.state === "installed" && (
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
              {running > 1 && <p className="agent-approval">Running in {running} terminals</p>}
              <button
                type="button"
                className="button-primary agent-launch"
                disabled={!info || agent.availability.state !== "installed"}
                onClick={() => actions.launchAgent(agent.id)}
              >
                Launch
              </button>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

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

function availabilityOf(agent: AgentStatus): Shown {
  return agent.availability.state === "installed" ? "installed" : agent.availability.state;
}
