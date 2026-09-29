import { type FormEvent, useEffect, useState } from "react";

import type { ProviderKind } from "../contracts/generated/ProviderKind";
import type { ProviderStatus } from "../contracts/generated/ProviderStatus";
import { useStore } from "../lib/useStore";
import type { ModelActions } from "./actions";
import type { Providers } from "./providers";

interface Props {
  providers: Providers;
  actions: ModelActions;
}

const HOSTING: Record<ProviderKind, string> = {
  hosted: "Hosted API",
  gateway: "Gateway to many providers' models",
  local: "Runs on this machine",
};

const SOURCES = { builtIn: "", local: "on this machine", custom: "added by you" } as const;

/** Model providers: keys, local availability and models (⇧⌘M). */
export function ModelsView({ providers, actions }: Props) {
  const { providers: list, loading, error } = useStore(providers);

  useEffect(() => {
    // Opening the view is asking: this is when Ollama is looked for on this machine.
    if (providers.get().providers === null) void providers.load(true);
  }, [providers]);

  return (
    <div className="agents">
      <header className="explorer-header">
        <span className="explorer-title">Models</span>
        <button
          type="button"
          className="icon-button"
          title="Look for local models again"
          aria-label="Refresh providers"
          onClick={actions.refreshProviders}
        >
          ↻
        </button>
      </header>
      <p className="agents-note">
        API keys are kept in your macOS Keychain and given only to agents you start with that provider. A saved key is never
        shown again. Choose a model when you launch an agent.
      </p>
      {error && (
        <p className="agents-note agents-error" role="alert">
          {error}
        </p>
      )}
      {list === null && loading && <p className="agents-note">Loading providers…</p>}
      <ul className="agents-list" aria-label="Model providers">
        {list?.map((provider) => (
          <ProviderCard key={provider.id} provider={provider} actions={actions} />
        ))}
      </ul>
    </div>
  );
}

function ProviderCard({ provider, actions }: { provider: ProviderStatus; actions: ModelActions }) {
  const [key, setKey] = useState("");
  const [model, setModel] = useState("");
  const [busy, setBusy] = useState(false);

  const saveKey = async (event: FormEvent) => {
    event.preventDefault();
    if (!key.trim() || busy) return;
    setBusy(true);
    const saved = await actions.saveProviderKey(provider.id, key);
    setBusy(false);
    // Out of the page as soon as it is in the Keychain.
    if (saved) setKey("");
  };
  const addModel = async (event: FormEvent) => {
    event.preventDefault();
    if (!model.trim()) return;
    if (await actions.addProviderModel(provider.id, model)) setModel("");
  };
  const [status, good] = statusOf(provider);

  return (
    <li className="agent" aria-label={`${provider.name} provider`}>
      <div className="agent-heading">
        <span className="agent-name">{provider.name}</span>
        <span className={good ? "agent-status agent-status-running" : "agent-status"}>{status}</span>
      </div>
      {provider.description && <p className="agent-description">{provider.description}</p>}
      <p className="agent-approval">{HOSTING[provider.hosting]}</p>
      <LocalNote provider={provider} />
      {provider.credential === "inKeychain" && (
        <p className="agent-approval">
          API key saved in your Keychain ·{" "}
          <button type="button" className="link-button" onClick={() => actions.removeProviderKey(provider.id)}>
            Remove
          </button>
        </p>
      )}
      {provider.credential === "missing" && (
        <form className="model-form" onSubmit={(e) => void saveKey(e)}>
          <input
            className="model-input"
            type="password"
            name={`${provider.id}-api-key`}
            autoComplete="off"
            spellCheck={false}
            placeholder={`${provider.name} API key`}
            aria-label={`${provider.name} API key`}
            value={key}
            onChange={(e) => setKey(e.target.value)}
          />
          <button type="submit" disabled={!key.trim() || busy}>
            Save
          </button>
        </form>
      )}
      <ModelList provider={provider} actions={actions} />
      <form className="model-form" onSubmit={(e) => void addModel(e)}>
        <input
          className="model-input"
          type="text"
          autoComplete="off"
          spellCheck={false}
          placeholder="Add a model id"
          aria-label={`Add a ${provider.name} model id`}
          value={model}
          onChange={(e) => setModel(e.target.value)}
        />
        <button type="submit" disabled={!model.trim()}>
          Add
        </button>
      </form>
    </li>
  );
}

function ModelList({ provider, actions }: { provider: ProviderStatus; actions: ModelActions }) {
  if (provider.models.length === 0) {
    return (
      <p className="agent-approval">
        {provider.hosting === "local"
          ? "No models found here. Pull one with Ollama yourself (ollama pull <model>), then refresh."
          : `The app lists no ${provider.name} models; add the ids you use.`}
      </p>
    );
  }
  return (
    <ul className="model-list" aria-label={`${provider.name} models`}>
      {provider.models.map((model) => (
        <li key={model.id} className="model-item">
          <span className="model-name" title={model.id}>
            {model.name}
            {model.name !== model.id && <span className="model-id"> {model.id}</span>}
          </span>
          {SOURCES[model.source] && <span className="model-source">{SOURCES[model.source]}</span>}
          {model.source === "custom" && (
            <button
              type="button"
              className="icon-button"
              title="Remove this model id"
              aria-label={`Remove ${model.id}`}
              onClick={() => actions.removeProviderModel(provider.id, model.id)}
            >
              ×
            </button>
          )}
        </li>
      ))}
    </ul>
  );
}

function LocalNote({ provider }: { provider: ProviderStatus }) {
  if (provider.hosting !== "local") return null;
  switch (provider.local?.state) {
    case undefined:
      return <p className="agent-approval">Not checked yet. Refresh to look for it.</p>;
    case "available":
      return <p className="agent-approval">Running{provider.local.version && ` (version ${provider.local.version})`}.</p>;
    case "installed":
      return <p className="agent-approval">Installed, but not running. Start Ollama, then refresh.</p>;
    case "unavailable":
      return <p className="agent-approval">Not found on this machine. Install it yourself if you want local models.</p>;
  }
}

/** The badge: what the provider needs next, or that it is ready. */
function statusOf(provider: ProviderStatus): [string, boolean] {
  if (provider.credential === "inKeychain") return ["Key saved", true];
  if (provider.credential === "missing") return ["No key", false];
  switch (provider.local?.state) {
    case "available":
      return ["Available", true];
    case "installed":
      return ["Installed", false];
    case "unavailable":
      return ["Unavailable", false];
    default:
      return ["Not checked", false];
  }
}
