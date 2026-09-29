import { AgentsView } from "../agents/AgentsView";
import { useStore } from "../lib/useStore";
import type { SidebarView } from "../workbench/layout";
import type { Workbench } from "../workbench/workbench";
import { FileExplorer } from "../workspace/FileExplorer";
import { SearchView } from "../workspace/SearchView";

const VIEWS: readonly { view: SidebarView; label: string; shortcut: string }[] = [
  { view: "files", label: "Files", shortcut: "⇧⌘E" },
  { view: "search", label: "Search", shortcut: "⇧⌘F" },
  { view: "agents", label: "Agents", shortcut: "⇧⌘A" },
];

/** The left column: the file explorer, search or agents, switched by tabs at the top. */
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
          <AgentsView agents={workbench.agents} terminals={workbench.terminals} workspace={workbench.workspace} actions={workbench} />
        </aside>
      )}
    </div>
  );
}
