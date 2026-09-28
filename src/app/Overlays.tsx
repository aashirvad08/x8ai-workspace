import { useEffect, useMemo, useRef, useState } from "react";

import { basename, dirname } from "../lib/paths";
import { useStore } from "../lib/useStore";
import { shortcutLabel } from "../workbench/commands";
import type { Dialogs } from "../workbench/dialogs";
import { fuzzyFilter } from "../workbench/fuzzy";
import type { Notifications } from "../workbench/notifications";
import type { Picker, PickerState } from "../workbench/picker";

export function NotificationList({ notifications }: { notifications: Notifications }) {
  const all = useStore(notifications);
  if (all.length === 0) return null;
  return (
    <div className="notifications" aria-live="polite">
      {all.map((n) => (
        <div key={n.id} className={`notification notification-${n.tone}`} role={n.tone === "error" ? "alert" : "status"}>
          <p>{n.message}</p>
          <div className="notification-actions">
            {n.actions.map((action) => (
              <button
                key={action.label}
                type="button"
                onClick={() => {
                  notifications.dismiss(n.id);
                  action.run();
                }}
              >
                {action.label}
              </button>
            ))}
            <button type="button" onClick={() => notifications.dismiss(n.id)}>
              Dismiss
            </button>
          </div>
        </div>
      ))}
    </div>
  );
}

export function DialogHost({ dialogs }: { dialogs: Dialogs }) {
  const dialog = useStore(dialogs);
  const primary = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    if (!dialog) return;
    // Focus goes back where it was when the dialog closes, so the next shortcut
    // acts on the same thing (⌘W in a terminal closes a pane, not an editor tab).
    const previous = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    primary.current?.focus();
    return () => previous?.focus();
  }, [dialog]);

  if (!dialog) return null;
  return (
    <div className="overlay" onPointerDown={() => dialogs.answer(dialog.cancel)}>
      <div
        className="dialog"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby="dialog-title"
        aria-describedby="dialog-message"
        onPointerDown={(event) => event.stopPropagation()}
        onKeyDown={(event) => {
          if (event.key === "Escape") dialogs.answer(dialog.cancel);
        }}
      >
        <h2 id="dialog-title">{dialog.title}</h2>
        <p id="dialog-message">{dialog.message}</p>
        <div className="dialog-buttons">
          {dialog.buttons.map((button) => (
            <button
              key={button.value}
              ref={button.role === "primary" || (!dialog.buttons.some((b) => b.role === "primary") && button.value === dialog.cancel) ? primary : undefined}
              type="button"
              className={button.role ? `button-${button.role}` : undefined}
              onClick={() => dialogs.answer(button.value)}
            >
              {button.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

interface PickerProps {
  picker: Picker;
  onOpenFile: (path: string) => void;
  onOpenWorkspace: (root: string) => void;
}

/** Quick open (⌘P), the command palette (⇧⌘P) and recent folders (⌃R). */
export function PickerView({ picker, onOpenFile, onOpenWorkspace }: PickerProps) {
  const state = useStore(picker);
  if (!state) return null;
  // A fresh query and selection each time the picker opens in a different mode.
  return <PickerDialog key={state.kind} state={state} picker={picker} onOpenFile={onOpenFile} onOpenWorkspace={onOpenWorkspace} />;
}

interface Item {
  key: string;
  primary: string;
  secondary: string;
  choose: () => void;
}

function PickerDialog({ state, picker, onOpenFile, onOpenWorkspace }: PickerProps & { state: PickerState }) {
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);

  const items = useMemo<Item[]>(() => {
    if (state.kind === "files") {
      return fuzzyFilter(query, state.files ?? [], (path) => path, 100).map((path) => ({
        key: path,
        primary: basename(path),
        secondary: dirname(path),
        choose: () => onOpenFile(path),
      }));
    }
    if (state.kind === "workspaces") {
      return fuzzyFilter(query, state.workspaces, (w) => w.root, 100).map((w) => ({
        key: w.root,
        primary: w.name,
        secondary: w.available ? w.root : `${w.root} (not found)`,
        choose: () => onOpenWorkspace(w.root),
      }));
    }
    return fuzzyFilter(query, state.commands, (command) => command.title, 100).map((command) => ({
      key: command.id,
      primary: command.title,
      secondary: command.shortcut ? shortcutLabel(command.shortcut) : "",
      choose: command.run,
    }));
  }, [state, query, onOpenFile, onOpenWorkspace]);

  const choose = (item: Item | undefined) => {
    if (!item) return;
    picker.close();
    item.choose();
  };

  const placeholder = { files: "Go to file…", commands: "Run a command…", workspaces: "Open recent folder…" }[state.kind];
  const empty =
    state.kind === "files" && state.files === null ? "Listing files…" : items.length === 0 ? "No matches" : null;

  return (
    <div className="overlay overlay-top" onPointerDown={() => picker.close()}>
      <div className="picker" role="dialog" aria-label={placeholder} onPointerDown={(event) => event.stopPropagation()}>
        <input
          autoFocus
          className="picker-input"
          placeholder={placeholder}
          aria-label={placeholder}
          spellCheck={false}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setIndex(0);
          }}
          onKeyDown={(event) => {
            if (event.key === "ArrowDown") setIndex((i) => Math.min(items.length - 1, i + 1));
            else if (event.key === "ArrowUp") setIndex((i) => Math.max(0, i - 1));
            else if (event.key === "Enter") choose(items[index]);
            else if (event.key === "Escape") picker.close();
            else return;
            event.preventDefault();
          }}
        />
        <ul className="picker-list" role="listbox">
          {empty && <li className="picker-empty">{empty}</li>}
          {items.map((item, i) => (
            <li
              key={item.key}
              role="option"
              aria-selected={i === index}
              className={i === index ? "picker-item picker-selected" : "picker-item"}
              onPointerEnter={() => setIndex(i)}
              onClick={() => choose(item)}
              ref={i === index ? (el) => el?.scrollIntoView({ block: "nearest" }) : undefined}
            >
              <span className="picker-primary">{item.primary}</span>
              <span className="picker-secondary">{item.secondary}</span>
            </li>
          ))}
        </ul>
        {state.kind === "files" && state.truncated && (
          <p className="picker-note">Showing the first {state.files?.length.toLocaleString()} files of a very large folder.</p>
        )}
      </div>
    </div>
  );
}
