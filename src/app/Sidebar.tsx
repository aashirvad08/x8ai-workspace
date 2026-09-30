import { AgentsView } from "../agents/AgentsView";
import { CatalogView } from "../catalog/CatalogView";
import { useStore } from "../lib/useStore";
import { McpView } from "../mcp/McpView";
import { ModelsView } from "../models/ModelsView";
import type { SidebarView } from "../workbench/layout";
import type { Workbench } from "../workbench/workbench";
import { FileExplorer } from "../workspace/FileExplorer";
import { SearchView } from "../workspace/SearchView";

const VIEWS: readonly { view: SidebarView; label: string; shortcut: string }[] = [
  { view: "files", label: "Files", shortcut: "⇧⌘E" },
  { view: "search", label: "Search", shortcut: "⇧⌘F" },
  { view: "agents", label: "Agents", shortcut: "⇧⌘A" },
  { view: "models", label: "Models", shortcut: "⇧⌘M" },
  { view: "mcp", label: "MCP", shortcut: "⇧⌘U" },
  { view: "catalog", label: "Catalog", shortcut: "⇧⌘K" },
];

/** The left column: the file explorer, search, agents, models, MCP servers or the catalog, switched by tabs at the top. */
export function Sidebar({ workbench }: { workbench: Workbench }) {
  const { sidebar } = useStore(workbench.layout);
  return (
    <div className="sidebar">
      <div className="sidebar-tabs" role="tablist" aria-label="Sidebar">
        {VIEWS.map(({ view, label, shortcut }) => (
          <button
            key={view}
            type="button"
            role="tab"
            aria-selected={sidebar === view}
            className={sidebar === view ? "sidebar-tab sidebar-tab-active" : "sidebar-tab"}
            title={`${label} (${shortcut})`}
            onClick={() => {
              if (view === "search") workbench.showSearch();
              else if (view === "agents") workbench.showAgents();
              else if (view === "models") workbench.showModels();
              else if (view === "mcp") workbench.showMcp();
              else if (view === "catalog") workbench.showCatalog();
              else workbench.layout.showSidebar(view);
            }}
          >
            {label}
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
