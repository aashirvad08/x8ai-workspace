import { type FormEvent, useEffect, useState } from "react";

import type { McpEnvSource } from "../contracts/generated/McpEnvSource";
import type { McpScope } from "../contracts/generated/McpScope";
import type { McpScopeKind } from "../contracts/generated/McpScopeKind";
import type { McpServerInput } from "../contracts/generated/McpServerInput";
import type { McpServerStatus } from "../contracts/generated/McpServerStatus";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import type { Store } from "../lib/store";
import { useStore } from "../lib/useStore";
import type { Agents } from "../agents/agents";
import type { McpActions } from "./actions";
import type { McpServers } from "./servers";

interface Props {
  mcp: McpServers;
  agents: Agents;
  workspace: Store<WorkspaceInfo | null>;
  actions: McpActions;
}

/** MCP servers: what the app may give agent sessions (⇧⌘U). */
export function McpView({ mcp, agents, workspace, actions }: Props) {
  const { servers, loading, error } = useStore(mcp);
  const { agents: agentList } = useStore(agents);
  const info = useStore(workspace);
  const agentName = (id: string) => agentList?.find((a) => a.id === id)?.name ?? id;
  const [editing, setEditing] = useState<string | "new" | null>(null);

  useEffect(() => {
    if (mcp.get().servers === null) void mcp.load();
    if (agents.get().agents === null) void agents.load();
  }, [mcp, agents]);

  return (
    <div className="agents">
      <header className="explorer-header">
        <span className="explorer-title">MCP Servers</span>
        <button type="button" className="icon-button" title="Add an MCP server" aria-label="Add MCP server" onClick={() => setEditing("new")}>
          +
        </button>
        <button type="button" className="icon-button" title="Check again" aria-label="Refresh MCP servers" onClick={() => actions.refreshMcp()}>
          ↻
        </button>
      </header>
      <p className="agents-note">
        The app gives these servers to agent sessions, in folders you trust, once you allow them there. Stdio servers are
        started by the app for a running session and stopped with it. Secrets are kept in your macOS Keychain.
      </p>
      {error && (
        <p className="agents-note agents-error" role="alert">
          {error}
        </p>
      )}
      {editing === "new" && (
        <ServerForm
          initial={null}
          workspaceName={info?.name ?? null}
          onSave={async (input) => (await actions.addMcpServer(input)) && (setEditing(null), true)}
          onCancel={() => setEditing(null)}
        />
      )}
      {servers === null && loading && <p className="agents-note">Loading MCP servers…</p>}
      {servers?.length === 0 && editing !== "new" && <p className="agents-note">No MCP servers yet. Add one with +.</p>}
      <ul className="agents-list" aria-label="MCP servers">
        {servers?.map((status) =>
          editing === status.server.id ? (
            <li key={status.server.id} className="agent">
              <ServerForm
                initial={status}
                workspaceName={info?.name ?? null}
                onSave={async (input) => (await actions.updateMcpServer(status.server.id, input)) && (setEditing(null), true)}
                onCancel={() => setEditing(null)}
              />
            </li>
          ) : (
            <ServerCard
              key={status.server.id}
              status={status}
              agentName={agentName}
              actions={actions}
              onEdit={() => setEditing(status.server.id)}
            />
          ),
        )}
      </ul>
    </div>
  );
}

function scopeLabel(scope: McpScope): string {
  switch (scope.kind) {
    case "global":
      return "Every session";
    case "workspace":
      return `Sessions in ${scope.root.split("/").pop() ?? scope.root}`;
    case "session":
      return "Chosen at launch";
  }
}

function ServerCard({
  status,
  agentName,
  actions,
  onEdit,
}: {
  status: McpServerStatus;
  agentName: (id: string) => string;
  actions: McpActions;
  onEdit: () => void;
}) {
  const { server } = status;
  const transport = server.transport;
  return (
    <li className="agent" aria-label={`${server.name} MCP server`}>
      <div className="agent-heading">
        <span className="agent-name">{server.name}</span>
        <span className={status.configured ? "agent-status agent-status-running" : "agent-status"}>
          {!server.enabled ? "Disabled" : status.configured ? "Ready" : "Not configured"}
        </span>
      </div>
      {server.description && <p className="agent-description">{server.description}</p>}
      <p className="agent-approval">
        {transport.kind === "stdio" ? "stdio" : "HTTP"} · {scopeLabel(server.scope)}
      </p>
      <p className="agent-path" title={transport.kind === "stdio" ? [transport.command, ...transport.args].join(" ") : transport.url}>
        {transport.kind === "stdio" ? [transport.command, ...transport.args].join(" ") : transport.url}
      </p>
      {status.problem && <p className="agent-approval agents-error">{status.problem}</p>}
      <p className="agent-approval">
        {status.agents.map((a, i) => (
          <span key={a.agent} title={a.reason ?? undefined}>
            {i > 0 && " · "}
            {agentName(a.agent)}: {a.supported ? "supported" : "not supported"}
          </span>
        ))}
      </p>
      {server.env
        .filter((v) => v.source === "inherit")
        .map((v) => (
          <p key={v.name} className="agent-approval">
            <code>{v.name}</code> from your shell
          </p>
        ))}
      {status.secrets.map((secret) => (
        <SecretRow key={secret.name} id={server.id} name={secret.name} saved={secret.state === "inKeychain"} actions={actions} />
      ))}
      <label className="agent-model">
        <input
          type="checkbox"
          checked={server.enabled}
          onChange={(e) => actions.setMcpServerEnabled(server.id, e.target.checked)}
          aria-label={`Enable ${server.name}`}
        />
        <span>Enabled</span>
      </label>
      <div className="agent-actions">
        <button type="button" onClick={onEdit}>
          Edit
        </button>
        <button type="button" onClick={() => actions.removeMcpServer(server.id)}>
          Remove
        </button>
      </div>
    </li>
  );
}

function SecretRow({ id, name, saved, actions }: { id: string; name: string; saved: boolean; actions: McpActions }) {
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const save = async (event: FormEvent) => {
    event.preventDefault();
    if (!value.trim() || busy) return;
    setBusy(true);
    const done = await actions.saveMcpSecret(id, name, value);
    setBusy(false);
    // Out of the page as soon as it is in the Keychain.
    if (done) setValue("");
  };
  if (saved) {
    return (
      <p className="agent-approval">
        <code>{name}</code> saved in your Keychain ·{" "}
        <button type="button" className="link-button" onClick={() => actions.removeMcpSecret(id, name)}>
          Remove
        </button>
      </p>
    );
  }
  return (
    <form className="model-form" onSubmit={(e) => void save(e)}>
      <input
        className="model-input"
        type="password"
        autoComplete="off"
        spellCheck={false}
        placeholder={name}
        aria-label={`${name} secret`}
        value={value}
        onChange={(e) => setValue(e.target.value)}
      />
      <button type="submit" disabled={!value.trim() || busy}>
        Save
      </button>
    </form>
  );
}

interface Variable {
  name: string;
  source: McpEnvSource;
}

function ServerForm({
  initial,
  workspaceName,
  onSave,
  onCancel,
}: {
  initial: McpServerStatus | null;
  workspaceName: string | null;
  onSave: (input: McpServerInput) => Promise<boolean>;
  onCancel: () => void;
}) {
  const server = initial?.server;
  const [name, setName] = useState(server?.name ?? "");
  const [description, setDescription] = useState(server?.description ?? "");
  const [kind, setKind] = useState<"stdio" | "streamableHttp">(server?.transport.kind ?? "stdio");
  const [command, setCommand] = useState(server?.transport.kind === "stdio" ? server.transport.command : "");
  const [args, setArgs] = useState(server?.transport.kind === "stdio" ? server.transport.args.join("\n") : "");
  const [url, setUrl] = useState(server?.transport.kind === "streamableHttp" ? server.transport.url : "");
  const [variables, setVariables] = useState<Variable[]>(server?.env.map((v) => ({ ...v })) ?? []);
  const [scope, setScope] = useState<McpScopeKind>(server?.scope.kind ?? "global");
  const [enabled, setEnabled] = useState(server?.enabled ?? true);
  const [busy, setBusy] = useState(false);

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    setBusy(true);
    await onSave({
      name,
      description,
      transport:
        kind === "stdio"
          ? { kind: "stdio", command: command.trim(), args: args.split("\n").filter((a) => a.trim() !== "") }
          : { kind: "streamableHttp", url: url.trim() },
      env: kind === "stdio" ? variables.filter((v) => v.name.trim() !== "").map((v) => ({ name: v.name.trim(), source: v.source })) : [],
      enabled,
      scope,
    });
    setBusy(false);
  };

  return (
    <form className="mcp-form" onSubmit={(e) => void submit(e)} aria-label={server ? `Edit ${server.name}` : "New MCP server"}>
      <label>
        Name
        <input className="model-input" value={name} onChange={(e) => setName(e.target.value)} aria-label="Server name" />
      </label>
      <label>
        Description
        <input className="model-input" value={description} onChange={(e) => setDescription(e.target.value)} aria-label="Server description" />
      </label>
      <label>
        Transport
        <select className="model-select" value={kind} onChange={(e) => setKind(e.target.value as typeof kind)} aria-label="Transport">
          <option value="stdio">stdio: a program on this Mac</option>
          <option value="streamableHttp">HTTP: a server at a URL</option>
        </select>
      </label>
      {kind === "stdio" ? (
        <>
          <label>
            Command
            <input
              className="model-input"
              value={command}
              spellCheck={false}
              placeholder="npx, or /absolute/path/to/server"
              onChange={(e) => setCommand(e.target.value)}
              aria-label="Command"
            />
          </label>
          <label>
            Arguments, one per line (passed as they are, never through a shell)
            <textarea className="model-input mcp-args" value={args} spellCheck={false} rows={3} onChange={(e) => setArgs(e.target.value)} aria-label="Arguments" />
          </label>
          <fieldset className="mcp-variables">
            <legend>Variables (names only; values are saved as secrets or taken from your shell)</legend>
            {variables.map((v, i) => (
              <div key={i} className="model-form">
                <input
                  className="model-input"
                  value={v.name}
                  spellCheck={false}
                  placeholder="NAME"
                  aria-label={`Variable ${i + 1} name`}
                  onChange={(e) => setVariables(variables.map((w, j) => (j === i ? { ...w, name: e.target.value } : w)))}
                />
                <select
                  className="model-select"
                  value={v.source}
                  aria-label={`Variable ${i + 1} source`}
                  onChange={(e) => setVariables(variables.map((w, j) => (j === i ? { ...w, source: e.target.value as McpEnvSource } : w)))}
                >
                  <option value="secret">Secret (Keychain)</option>
                  <option value="inherit">From your shell</option>
                </select>
                <button type="button" aria-label={`Remove variable ${i + 1}`} onClick={() => setVariables(variables.filter((_, j) => j !== i))}>
                  ×
                </button>
              </div>
            ))}
            <button type="button" className="link-button" onClick={() => setVariables([...variables, { name: "", source: "secret" }])}>
              Add a variable
            </button>
          </fieldset>
        </>
      ) : (
        <label>
          URL
          <input
            className="model-input"
            value={url}
            spellCheck={false}
            placeholder="https://mcp.example.com/mcp"
            onChange={(e) => setUrl(e.target.value)}
            aria-label="URL"
          />
        </label>
      )}
      <label>
        Attach to
        <select className="model-select" value={scope} onChange={(e) => setScope(e.target.value as McpScopeKind)} aria-label="Scope">
          <option value="global">Every new session</option>
          <option value="workspace" disabled={!workspaceName && server?.scope.kind !== "workspace"}>
            New sessions in {workspaceName ? `“${workspaceName}”` : "this folder"}
          </option>
          <option value="session">Only when chosen at launch</option>
        </select>
      </label>
      <label className="agent-model">
        <input type="checkbox" checked={enabled} onChange={(e) => setEnabled(e.target.checked)} aria-label="Enabled" />
        <span>Enabled</span>
      </label>
      <div className="agent-actions">
        <button type="submit" disabled={busy || !name.trim()}>
          {server ? "Save" : "Add"}
        </button>
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
