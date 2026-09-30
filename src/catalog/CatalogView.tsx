import { type FormEvent, useEffect, useMemo, useState } from "react";

import type { CatalogItem } from "../contracts/generated/CatalogItem";
import type { CatalogSource } from "../contracts/generated/CatalogSource";
import type { CatalogStatus } from "../contracts/generated/CatalogStatus";
import type { McpScopeKind } from "../contracts/generated/McpScopeKind";
import type { SkillInput } from "../contracts/generated/SkillInput";
import type { Skill } from "../contracts/generated/Skill";
import { useStore } from "../lib/useStore";
import type { Skills } from "../skills/skills";
import type { CatalogActions } from "./actions";
import { type Catalog, type CatalogCategory, filterCatalog, type StatusFilter, TYPE_LABELS } from "./catalog";

interface Props {
  catalog: Catalog;
  skills: Skills;
  actions: CatalogActions;
}

const CATEGORIES: readonly { category: CatalogCategory; label: string }[] = [
  { category: "all", label: "All" },
  { category: "agent", label: "Agents" },
  { category: "model", label: "Models" },
  { category: "mcpServer", label: "MCP" },
  { category: "skill", label: "Skills" },
];

const STATUS_LABELS: Record<CatalogStatus, string> = {
  installed: "Installed",
  available: "Available",
  configured: "Configured",
  unsupported: "Unsupported",
  unavailable: "Unavailable",
};

const SOURCE_LABELS: Record<CatalogSource, string> = {
  builtin: "Built in",
  local: "Found on this Mac",
  userDefined: "Added by you",
  remote: "Remote",
};

/** Discover agents, models, MCP servers and skills (⇧⌘K). Nothing here runs anything. */
export function CatalogView({ catalog, skills, actions }: Props) {
  const { items, loading, error, warnings } = useStore(catalog);
  const { skills: skillList } = useStore(skills);
  const [text, setText] = useState("");
  const [category, setCategory] = useState<CatalogCategory>("all");
  const [status, setStatus] = useState<StatusFilter>("any");
  const [open, setOpen] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | "new" | null>(null);

  useEffect(() => {
    // Opened with ⇧⌘K it is loading already; restored at startup it is not.
    const { items: loaded, loading: busy } = catalog.get();
    if (loaded === null && !busy) void catalog.load();
    if (skills.get().skills === null) void skills.load();
  }, [catalog, skills]);

  const shown = useMemo(() => filterCatalog(items ?? [], { text, category, status }), [items, text, category, status]);
  const editingSkill = editing && editing !== "new" ? skillList?.find((s) => s.skill.id === editing)?.skill : undefined;

  return (
    <div className="agents catalog">
      <header className="explorer-header">
        <span className="explorer-title">Catalog</span>
        <button type="button" className="icon-button" title="Look again" aria-label="Refresh the catalog" onClick={() => actions.refreshCatalog()}>
          ↻
        </button>
      </header>
      <p className="agents-note">
        What the app knows and what each part needs. Nothing here installs, runs or goes online; setting up and allowing
        stay with each part.
      </p>
      <div className="search-field catalog-search">
        <input
          className="search-input"
          type="search"
          placeholder="Search the catalog"
          aria-label="Search the catalog"
          spellCheck={false}
          value={text}
          onChange={(e) => setText(e.target.value)}
        />
      </div>
      <div className="catalog-filters" role="group" aria-label="Category">
        {CATEGORIES.map((c) => (
          <button
            key={c.category}
            type="button"
            aria-pressed={category === c.category}
            className={category === c.category ? "search-option search-option-on" : "search-option"}
            onClick={() => setCategory(c.category)}
          >
            {c.label}
          </button>
        ))}
        <select className="model-select catalog-status" value={status} aria-label="Status filter" onChange={(e) => setStatus(e.target.value as StatusFilter)}>
          <option value="any">Any status</option>
          <option value="usable">Installed or configured</option>
          <option value="installed">Installed</option>
          <option value="configured">Configured</option>
        </select>
      </div>
      {error && (
        <p className="agents-note agents-error" role="alert">
          {error}
        </p>
      )}
      {warnings.map((w) => (
        <p key={w} className="agents-note agents-note-warning">
          {w}
        </p>
      ))}
      {(category === "skill" || category === "all") && editing === null && (
        <button type="button" className="link-button catalog-new-skill" onClick={() => setEditing("new")}>
          New skill…
        </button>
      )}
      {editing !== null && (
        <SkillForm
          initial={editingSkill ?? null}
          onSave={async (input) => {
            const saved = editingSkill ? await actions.updateSkill(editingSkill.id, input) : await actions.addSkill(input);
            if (saved) setEditing(null);
          }}
          onCancel={() => setEditing(null)}
        />
      )}
      {items === null && loading && <p className="agents-note">Loading the catalog…</p>}
      {items !== null && shown.length === 0 && <p className="agents-note">Nothing matches.</p>}
      <ul className="agents-list" aria-label="Catalog items">
        {shown.map((item) => (
          <CatalogRow
            key={item.id}
            item={item}
            open={open === item.id}
            onToggle={() => setOpen(open === item.id ? null : item.id)}
            onEditSkill={(id) => setEditing(id)}
            actions={actions}
          />
        ))}
      </ul>
    </div>
  );
}

function statusClass(status: CatalogStatus): string {
  return status === "installed" || status === "configured" ? "agent-status agent-status-running" : "agent-status";
}

function CatalogRow({
  item,
  open,
  onToggle,
  onEditSkill,
  actions,
}: {
  item: CatalogItem;
  open: boolean;
  onToggle: () => void;
  onEditSkill: (id: string) => void;
  actions: CatalogActions;
}) {
  const by = item.details.kind === "model" ? item.details.providerName : item.publisher;
  return (
    <li className="agent catalog-item" aria-label={`${item.displayName} ${TYPE_LABELS[item.type]}`}>
      <button type="button" className="catalog-row" aria-expanded={open} onClick={onToggle}>
        <span className="agent-heading">
          <span className="agent-name">{item.displayName}</span>
          <span className={statusClass(item.status)}>{STATUS_LABELS[item.status]}</span>
        </span>
        <span className="agent-approval">
          {TYPE_LABELS[item.type]}
          {by && ` · ${by}`}
          {item.catalogVersion && ` · catalog v${item.catalogVersion}`}
        </span>
        {item.description && <span className="agent-description">{item.description}</span>}
      </button>
      {open && <CatalogDetails item={item} onEditSkill={onEditSkill} actions={actions} />}
    </li>
  );
}

function CatalogDetails({ item, onEditSkill, actions }: { item: CatalogItem; onEditSkill: (id: string) => void; actions: CatalogActions }) {
  const d = item.details;
  return (
    <div className="agent-changes catalog-details">
      {item.statusDetail && <p className="agent-approval">{item.statusDetail}</p>}
      <dl className="catalog-facts">
        <dt>Id</dt>
        <dd>
          <code>{item.id}</code>
        </dd>
        <dt>Source</dt>
        <dd>{SOURCE_LABELS[item.source]}</dd>
        {item.publisher && (
          <>
            <dt>Publisher</dt>
            <dd>{item.publisher}</dd>
          </>
        )}
        {d.kind === "model" && (
          <>
            <dt>Provider</dt>
            <dd>{d.providerName}</dd>
            <dt>Model id</dt>
            <dd>
              <code>{d.model}</code>
            </dd>
          </>
        )}
        {d.kind === "agent" && d.executable && (
          <>
            <dt>Program</dt>
            <dd>
              <code>{d.executable}</code>
            </dd>
          </>
        )}
        <dt>Catalog version</dt>
        <dd>{item.catalogVersion ?? "—"}</dd>
        <dt>Installed version</dt>
        <dd>{item.softwareVersion ?? "not checked (the app runs nothing to find out)"}</dd>
      </dl>
      {item.capabilities.length > 0 && <p className="agent-approval">Can: {item.capabilities.join(" · ")}</p>}
      {item.requirements.length > 0 && <p className="agent-approval">Needs: {item.requirements.join(" · ")}</p>}
      {item.tags.length > 0 && <p className="agent-approval">Tags: {item.tags.join(", ")}</p>}
      <div className="agent-actions">
        {d.kind === "agent" &&
          (item.status === "installed" ? (
            <button type="button" onClick={() => actions.openAgent(d.agent)}>
              Open in Agents
            </button>
          ) : (
            <span className="agent-approval">
              Not installed. The app does not install programs: install it yourself, then refresh.
            </span>
          ))}
        {d.kind === "provider" && (
          <button type="button" onClick={() => actions.configureProvider(d.provider)}>
            Set up in Models
          </button>
        )}
        {d.kind === "model" &&
          (item.status === "configured" ? (
            <button type="button" onClick={() => actions.chooseModel(d.provider, d.model)}>
              Use for the next launch
            </button>
          ) : (
            <button type="button" onClick={() => actions.configureProvider(d.provider)}>
              Set up {d.providerName}
            </button>
          ))}
        {d.kind === "mcpServer" && (
          <>
            {d.enabled && d.scope.kind === "session" && item.status === "configured" && (
              <button type="button" onClick={() => actions.attachMcp(d.server)}>
                Attach to the next launch
              </button>
            )}
            <button type="button" onClick={() => actions.setMcpServerEnabled(d.server, !d.enabled)}>
              {d.enabled ? "Disable" : "Enable"}
            </button>
            <button type="button" onClick={() => actions.configureMcp(d.server)}>
              Configure in MCP
            </button>
          </>
        )}
        {d.kind === "skill" && (
          <>
            {d.scope.kind === "session" && item.status === "installed" && (
              <button type="button" onClick={() => actions.attachSkill(d.skill)}>
                Attach to the next launch
              </button>
            )}
            {d.source === "user" && (
              <>
                <button type="button" onClick={() => onEditSkill(d.skill)}>
                  Edit
                </button>
                <button type="button" onClick={() => actions.removeSkill(d.skill)}>
                  Remove
                </button>
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function SkillForm({ initial, onSave, onCancel }: { initial: Skill | null; onSave: (input: SkillInput) => Promise<void>; onCancel: () => void }) {
  const [name, setName] = useState(initial?.name ?? "");
  const [description, setDescription] = useState(initial?.description ?? "");
  const [instructions, setInstructions] = useState(initial?.instructions ?? "");
  const [tools, setTools] = useState(initial?.allowedTools.join(", ") ?? "");
  const [scope, setScope] = useState<McpScopeKind>(initial?.scope.kind ?? "session");
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    await onSave({
      name,
      description,
      instructions,
      allowedTools: tools
        .split(",")
        .map((t) => t.trim())
        .filter(Boolean),
      scope,
    });
  };
  return (
    <form className="mcp-form catalog-skill-form" aria-label={initial ? `Edit ${initial.name}` : "New skill"} onSubmit={(e) => void submit(e)}>
      <label>
        Name
        <input className="model-input" value={name} onChange={(e) => setName(e.target.value)} aria-label="Skill name" />
      </label>
      <label>
        Description
        <input className="model-input" value={description} onChange={(e) => setDescription(e.target.value)} aria-label="Skill description" />
      </label>
      <label>
        Instructions (text the agent is given; never a secret)
        <textarea className="model-input mcp-args" rows={6} value={instructions} onChange={(e) => setInstructions(e.target.value)} aria-label="Skill instructions" />
      </label>
      <label>
        Suggested tools, comma separated (shown only; never granted)
        <input className="model-input" value={tools} spellCheck={false} onChange={(e) => setTools(e.target.value)} aria-label="Suggested tools" />
      </label>
      <label>
        Attach to
        <select className="model-select" value={scope} onChange={(e) => setScope(e.target.value as McpScopeKind)} aria-label="Skill scope">
          <option value="session">Only when chosen at launch</option>
          <option value="workspace">New sessions in this folder</option>
          <option value="global">Every new session</option>
        </select>
      </label>
      <div className="agent-actions">
        <button type="submit" disabled={!name.trim() || !instructions.trim()}>
          {initial ? "Save" : "Add"}
        </button>
        <button type="button" onClick={onCancel}>
          Cancel
        </button>
      </div>
    </form>
  );
}
