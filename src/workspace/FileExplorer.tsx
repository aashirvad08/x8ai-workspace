import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";

import type { DirEntry } from "../contracts/generated/DirEntry";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import { dirname } from "../lib/paths";
import type { Store } from "../lib/store";
import { useStore } from "../lib/useStore";
import type { ExplorerActions } from "./actions";
import { type Editing, type Explorer, type Row, visibleRows } from "./explorer";

interface Props {
  explorer: Explorer;
  workspace: Store<WorkspaceInfo | null>;
  actions: ExplorerActions;
}

interface Menu {
  x: number;
  y: number;
  /** The entry right-clicked, or `null` for the empty area (the root). */
  entry: DirEntry | null;
}

const INDENT = 14;

export function FileExplorer({ explorer, workspace, actions }: Props) {
  const info = useStore(workspace);
  const snapshot = useStore(explorer);
  const rows = useMemo(() => visibleRows(snapshot), [snapshot]);
  const [menu, setMenu] = useState<Menu | null>(null);
  const tree = useRef<HTMLDivElement>(null);

  if (!info) {
    return (
      <aside className="explorer" aria-label="Files">
        <div className="explorer-empty">
          <p>No folder open.</p>
          <button type="button" className="button-primary" onClick={actions.openFolder}>
            Open Folder…
          </button>
          <p className="hint">
            <kbd>⌘O</kbd>
          </p>
        </div>
      </aside>
    );
  }

  const entries = rows.flatMap((row) => (row.type === "entry" ? [row] : []));
  const selectedIndex = entries.findIndex((row) => row.entry.path === snapshot.selected);
  const selectedEntry = entries[selectedIndex]?.entry ?? null;

  const activate = (entry: DirEntry) => {
    explorer.select(entry.path);
    if (entry.kind === "directory") explorer.toggle(entry.path);
    else if (entry.kind === "file") actions.openFile(entry.path);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (snapshot.editing) return;
    const move = (index: number) => {
      const target = entries[Math.max(0, Math.min(entries.length - 1, index))];
      if (target) explorer.select(target.entry.path);
    };
    switch (event.key) {
      case "ArrowDown":
        move(selectedIndex + 1);
        break;
      case "ArrowUp":
        move(selectedIndex < 0 ? 0 : selectedIndex - 1);
        break;
      case "ArrowRight":
        if (selectedEntry?.kind === "directory" && !snapshot.expanded.has(selectedEntry.path)) {
          explorer.expand(selectedEntry.path);
        } else {
          move(selectedIndex + 1);
        }
        break;
      case "ArrowLeft":
        if (selectedEntry?.kind === "directory" && snapshot.expanded.has(selectedEntry.path)) {
          explorer.toggle(selectedEntry.path);
        } else if (selectedEntry && dirname(selectedEntry.path) !== "") {
          explorer.select(dirname(selectedEntry.path));
        }
        break;
      case "Enter":
        if (selectedEntry) activate(selectedEntry);
        break;
      case "F2":
        if (selectedEntry) explorer.startEditing({ kind: "rename", path: selectedEntry.path });
        break;
      case "Backspace":
        if (!event.metaKey || !selectedEntry) return;
        actions.remove(selectedEntry);
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  const openMenu = (event: MouseEvent, entry: DirEntry | null) => {
    event.preventDefault();
    event.stopPropagation();
    if (entry) explorer.select(entry.path);
    setMenu({ x: event.clientX, y: event.clientY, entry });
  };

  return (
    <aside className="explorer" aria-label="Files">
      <header className="explorer-header">
        <span className="explorer-title" title={info.root}>
          {info.name}
        </span>
        <IconButton label="New File (⌘N)" onClick={() => actions.startCreating("file")}>
          +
        </IconButton>
        <IconButton label="New Folder" onClick={() => actions.startCreating("folder")}>
          ⊞
        </IconButton>
        <IconButton label="Collapse Folders" onClick={() => explorer.collapseAll()}>
          ⊟
        </IconButton>
      </header>
      <div
        ref={tree}
        className="tree"
        role="tree"
        tabIndex={0}
        aria-activedescendant={snapshot.selected === null ? undefined : rowId(snapshot.selected)}
        onKeyDown={onKeyDown}
        onContextMenu={(event) => openMenu(event, null)}
      >
        {rows.map((row) => (
          <TreeRow
            key={rowKey(row)}
            row={row}
            snapshot={{ selected: snapshot.selected, editing: snapshot.editing }}
            explorer={explorer}
            actions={actions}
            onActivate={activate}
            onMenu={openMenu}
            onDone={() => tree.current?.focus()}
          />
        ))}
      </div>
      {menu && (
        <ContextMenu
          menu={menu}
          onClose={() => setMenu(null)}
          items={menuItems(menu.entry, explorer, actions)}
        />
      )}
    </aside>
  );
}

function TreeRow({
  row,
  snapshot,
  explorer,
  actions,
  onActivate,
  onMenu,
  onDone,
}: {
  row: Row;
  snapshot: { selected: string | null; editing: Editing | null };
  explorer: Explorer;
  actions: ExplorerActions;
  onActivate: (entry: DirEntry) => void;
  onMenu: (event: MouseEvent, entry: DirEntry) => void;
  onDone: () => void;
}) {
  const indent = { paddingLeft: 8 + row.depth * INDENT };

  if (row.type === "status") {
    return (
      <div className={row.error ? "tree-status tree-error" : "tree-status"} style={indent} role="none">
        {row.text}
      </div>
    );
  }

  if (row.type === "new") {
    const { editing } = row;
    if (editing.kind === "rename") return null;
    const kind = editing.kind === "newFile" ? "file" : "folder";
    return (
      <div className="tree-row" style={indent} role="none">
        <span className="tree-chevron" />
        <NameField
          initial=""
          placeholder={kind === "file" ? "File name" : "Folder name"}
          onSubmit={(name) => actions.create(editing.parent, name, kind)}
          onDone={() => {
            explorer.stopEditing();
            onDone();
          }}
        />
      </div>
    );
  }

  const { entry, expanded } = row;
  const renaming = snapshot.editing?.kind === "rename" && snapshot.editing.path === entry.path;
  const selected = snapshot.selected === entry.path;
  return (
    <div
      id={rowId(entry.path)}
      className={selected ? "tree-row tree-selected" : "tree-row"}
      style={indent}
      role="treeitem"
      aria-selected={selected}
      aria-expanded={entry.kind === "directory" ? expanded : undefined}
      title={entry.symlink ? `${entry.path} (symbolic link)` : entry.path}
      onClick={() => !renaming && onActivate(entry)}
      onContextMenu={(event) => onMenu(event, entry)}
    >
      <span className={expanded ? "tree-chevron tree-open" : "tree-chevron"}>
        {entry.kind === "directory" ? "›" : ""}
      </span>
      {renaming ? (
        <NameField
          initial={entry.name}
          placeholder="Name"
          onSubmit={(name) => actions.rename(entry.path, name)}
          onDone={() => {
            explorer.stopEditing();
            onDone();
          }}
        />
      ) : (
        <span className={entry.kind === "other" ? "tree-name tree-unusable" : "tree-name"}>
          {entry.name}
          {entry.symlink && <span className="tree-link"> ↗</span>}
        </span>
      )}
    </div>
  );
}

/** An inline text field. Enter or leaving the field submits; Escape cancels. */
function NameField({
  initial,
  placeholder,
  onSubmit,
  onDone,
}: {
  initial: string;
  placeholder: string;
  onSubmit: (name: string) => Promise<boolean>;
  onDone: () => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const settled = useRef(false);

  useEffect(() => {
    const field = input.current!;
    field.focus();
    // Select the name without its extension, as Finder does.
    const dot = initial.lastIndexOf(".");
    field.setSelectionRange(0, dot > 0 ? dot : initial.length);
  }, [initial]);

  const finish = async (submit: boolean) => {
    if (settled.current) return;
    const name = input.current!.value.trim();
    if (submit && name !== "" && name !== initial) {
      settled.current = true;
      const ok = await onSubmit(name);
      if (!ok) {
        // Keep the field so the name can be corrected; the error is shown.
        settled.current = false;
        input.current?.focus();
        return;
      }
    }
    settled.current = true;
    onDone();
  };

  return (
    <input
      ref={input}
      className="tree-input"
      defaultValue={initial}
      placeholder={placeholder}
      aria-label={placeholder}
      spellCheck={false}
      onClick={(event) => event.stopPropagation()}
      onKeyDown={(event) => {
        event.stopPropagation();
        if (event.key === "Enter") void finish(true);
        if (event.key === "Escape") void finish(false);
      }}
      onBlur={() => void finish(true)}
    />
  );
}

interface MenuItem {
  label: string;
  run: () => void;
  destructive?: boolean;
}

function menuItems(entry: DirEntry | null, explorer: Explorer, actions: ExplorerActions): MenuItem[][] {
  const create = (parent: string): MenuItem[] => [
    { label: "New File…", run: () => explorer.startEditing({ kind: "newFile", parent }) },
    { label: "New Folder…", run: () => explorer.startEditing({ kind: "newFolder", parent }) },
  ];
  if (!entry) return [create("")];
  const modify: MenuItem[] = [
    { label: "Rename…", run: () => explorer.startEditing({ kind: "rename", path: entry.path }) },
    { label: "Move to Trash", run: () => actions.remove(entry), destructive: true },
  ];
  if (entry.kind === "directory") return [create(entry.path), modify];
  return [[{ label: "Open", run: () => actions.openFile(entry.path) }], create(dirname(entry.path)), modify];
}

function ContextMenu({ menu, items, onClose }: { menu: Menu; items: MenuItem[][]; onClose: () => void }) {
  const element = useRef<HTMLDivElement>(null);

  useEffect(() => {
    element.current?.focus();
    const close = (event: Event) => {
      if (!element.current?.contains(event.target as Node)) onClose();
    };
    window.addEventListener("pointerdown", close, true);
    window.addEventListener("blur", onClose);
    return () => {
      window.removeEventListener("pointerdown", close, true);
      window.removeEventListener("blur", onClose);
    };
  }, [onClose]);

  return (
    <div
      ref={element}
      className="context-menu"
      role="menu"
      tabIndex={-1}
      style={{ left: menu.x, top: menu.y }}
      onKeyDown={(event) => event.key === "Escape" && onClose()}
    >
      {items.map((group, i) => (
        <div key={i} className="context-group">
          {group.map((item) => (
            <button
              key={item.label}
              type="button"
              role="menuitem"
              className={item.destructive ? "context-item context-destructive" : "context-item"}
              onClick={() => {
                onClose();
                item.run();
              }}
            >
              {item.label}
            </button>
          ))}
        </div>
      ))}
    </div>
  );
}

function IconButton({ label, onClick, children }: { label: string; onClick: () => void; children: string }) {
  return (
    <button type="button" className="icon-button" title={label} aria-label={label} onClick={onClick}>
      {children}
    </button>
  );
}

function rowKey(row: Row): string {
  if (row.type === "entry") return `e:${row.entry.path}`;
  if (row.type === "status") return `s:${row.dir}`;
  return `n:${row.editing.kind === "rename" ? row.editing.path : row.editing.parent}`;
}

function rowId(path: string): string {
  return `tree-${encodeURIComponent(path)}`;
}
