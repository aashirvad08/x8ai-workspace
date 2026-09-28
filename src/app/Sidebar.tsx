import { useStore } from "../lib/useStore";
import type { SidebarView } from "../workbench/layout";
import type { Workbench } from "../workbench/workbench";
import { FileExplorer } from "../workspace/FileExplorer";
import { SearchView } from "../workspace/SearchView";

const VIEWS: readonly { view: SidebarView; label: string; shortcut: string }[] = [
  { view: "files", label: "Files", shortcut: "⇧⌘E" },
  { view: "search", label: "Search", shortcut: "⇧⌘F" },
];

/** The left column: the file explorer or search, switched by tabs at the top. */
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
            onClick={() => (view === "search" ? workbench.showSearch() : workbench.layout.showSidebar(view))}
          >
            {label}
          </button>
        ))}
      </div>
      {sidebar === "files" ? (
        <FileExplorer explorer={workbench.explorer} workspace={workbench.workspace} recent={workbench.recent} actions={workbench} />
      ) : (
        <aside className="explorer" aria-label="Search">
          <SearchView search={workbench.search} workspace={workbench.workspace} actions={workbench} />
        </aside>
      )}
    </div>
  );
}
