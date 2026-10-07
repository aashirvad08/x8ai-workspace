import { AddonsView } from "../addons/AddonsView";
import { AgentsView } from "../agents/AgentsView";
import { CatalogView } from "../catalog/CatalogView";
import { useStore } from "../lib/useStore";
import { McpView } from "../mcp/McpView";
import { ModelsView } from "../models/ModelsView";
import type { SidebarView } from "../workbench/layout";
import type { Workbench } from "../workbench/workbench";
import { FileExplorer } from "../workspace/FileExplorer";
import { SearchView } from "../workspace/SearchView";

const VIEWS: readonly { view: SidebarView; label: string; shortcut: string; icon: string }[] = [
  { view: "files", label: "Files", shortcut: "⇧⌘E", icon: "M2 4.5h4l1.5 1.5H14v6.5H2z" },
  { view: "search", label: "Search", shortcut: "⇧⌘F", icon: "M11.2 7a4.2 4.2 0 1 1-8.4 0 4.2 4.2 0 0 1 8.4 0zM10.2 10.2l3.3 3.3" },
  { view: "agents", label: "Agents", shortcut: "⇧⌘A", icon: "M1.5 3.5h13v9h-13zM4.5 6.2l2 1.8-2 1.8M8 10.2h3.5" },
  {
    view: "models",
    label: "Models",
    shortcut: "⇧⌘M",
    icon: "M4.5 4.5h7v7h-7zM6.5 2v2.5M9.5 2v2.5M6.5 11.5V14M9.5 11.5V14M2 6.5h2.5M2 9.5h2.5M11.5 6.5H14M11.5 9.5H14",
  },
  { view: "mcp", label: "MCP", shortcut: "⇧⌘U", icon: "M6 2v3M10 2v3M4.5 5h7v2.5a3.5 3.5 0 0 1-7 0zM8 11v3" },
  { view: "catalog", label: "Catalog", shortcut: "⇧⌘K", icon: "M2.5 2.5h4v4h-4zM9.5 2.5h4v4h-4zM2.5 9.5h4v4h-4zM9.5 9.5h4v4h-4z" },
  { view: "addons", label: "Add-ons", shortcut: "⇧⌘X", icon: "M3 5.5h3.2a1.8 1.8 0 1 1 3.6 0H13v3.2a1.8 1.8 0 1 1 0 3.6V14H3z" },
];

/** The left column: the file explorer, search, agents, models, MCP servers, the catalog or add-ons, switched by tabs at the top. */
export function Sidebar({ workbench }: { workbench: Workbench }) {
  const { sidebar } = useStore(workbench.layout);
  return (
    <div className="sidebar">
      <div className="sidebar-tabs" role="tablist" aria-label="Sidebar">
        {VIEWS.map(({ view, label, shortcut, icon }) => (
          <button
            key={view}
            type="button"
            role="tab"
            aria-selected={sidebar === view}
            className={sidebar === view ? "sidebar-tab sidebar-tab-active" : "sidebar-tab"}
            title={`${label} (${shortcut})`}
            aria-label={label}
            onClick={() => {
              if (view === "search") workbench.showSearch();
              else if (view === "agents") workbench.showAgents();
              else if (view === "models") workbench.showModels();
              else if (view === "mcp") workbench.showMcp();
              else if (view === "catalog") workbench.showCatalog();
              else if (view === "addons") workbench.showAddons();
              else workbench.layout.showSidebar(view);
            }}
          >
            <svg className="sidebar-icon" viewBox="0 0 16 16" aria-hidden>
              <path d={icon} />
            </svg>
          </button>
        ))}
      </div>
      {sidebar === "files" && (
        <FileExplorer explorer={workbench.explorer} workspace={workbench.workspace} recent={workbench.recent} actions={workbench} />
      )}
      {sidebar === "search" && (
        <aside className="explorer" aria-label="Search">
          <SearchView search={workbench.search} workspace={workbench.workspace} actions={workbench} />
        </aside>
      )}
      {sidebar === "agents" && (
        <aside className="explorer" aria-label="Agents">
          <AgentsView
            agents={workbench.agents}
            providers={workbench.providers}
            mcp={workbench.mcp}
            skills={workbench.skills}
            drafts={workbench.drafts}
            terminals={workbench.terminals}
            workspace={workbench.workspace}
            actions={workbench}
          />
        </aside>
      )}
      {sidebar === "catalog" && (
        <aside className="explorer" aria-label="Catalog">
          <CatalogView catalog={workbench.catalog} skills={workbench.skills} actions={workbench} />
        </aside>
      )}
      {sidebar === "addons" && (
        <aside className="explorer" aria-label="Add-ons">
          <AddonsView addons={workbench.addons} actions={workbench} />
        </aside>
      )}
      {sidebar === "mcp" && (
        <aside className="explorer" aria-label="MCP servers">
          <McpView mcp={workbench.mcp} agents={workbench.agents} workspace={workbench.workspace} actions={workbench} />
        </aside>
      )}
      {sidebar === "models" && (
        <aside className="explorer" aria-label="Models">
          <ModelsView providers={workbench.providers} actions={workbench} />
        </aside>
      )}
    </div>
  );
}
