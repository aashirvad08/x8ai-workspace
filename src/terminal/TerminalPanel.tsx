import { type DragEvent, type KeyboardEvent, memo, type PointerEvent, type RefObject, useMemo, useRef, useState } from "react";

import { carriesModel, readModelDrag } from "../lib/modelDrag";
import { useStore } from "../lib/useStore";
import type { AddonApi, AgentApi, TerminalApi } from "../native";
import type { TerminalActions } from "./actions";
import { layoutPanes, type PaneLayout, paneKeys, type Rect, type SplitDirection } from "./panes";
import type { SessionNative } from "./session";
import { type PaneKind, type TerminalPane, type Terminals, tabTitle } from "./terminals";
import { DEFAULT_FONT, TerminalView } from "./TerminalView";

type PanelNative = TerminalApi & Pick<AgentApi, "runAgentSession"> & Pick<AddonApi, "installAddon">;

interface Props {
  native: PanelNative;
  terminals: Terminals;
  actions: TerminalActions;
  /** Collapsed to its tab bar; its sessions keep running. */
  collapsed: boolean;
  onHide: () => void;
  onShow: () => void;
}

const KEY_STEP = 0.05;

export function TerminalPanel({ native, terminals, actions, collapsed, onHide, onShow }: Props) {
  const { tabs, panes, active, focusRequest, font } = useStore(terminals);
  const body = useRef<HTMLDivElement>(null);
  const layouts = new Map<number, PaneLayout>(tabs.map((tab) => [tab.key, layoutPanes(tab.tree)]));
  // In creation order, whatever the layout, so splitting never moves an existing
  // view in the DOM (which would restart its renderer).
  const placed = tabs
    .flatMap((tab) => layouts.get(tab.key)!.panes.map((p) => ({ ...p, tab })))
    .sort((a, b) => a.key - b.key);
  const activeTab = tabs.find((tab) => tab.key === active);
  // A model from the catalog, dragged over: dropping it opens it in a terminal of its own.
  const [dropping, setDropping] = useState(false);
  const overModel = (e: DragEvent) => {
    if (!carriesModel(e.dataTransfer)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "copy";
    setDropping(true);
  };
  const dropModel = (e: DragEvent) => {
    setDropping(false);
    const dragged = readModelDrag(e.dataTransfer);
    if (!dragged) return;
    e.preventDefault();
    actions.launchModel(dragged.provider, dragged.model);
  };

  return (
    <section
      className={collapsed ? "terminal-panel terminal-panel-collapsed" : "terminal-panel"}
      aria-label="Terminal"
      onDragEnterCapture={overModel}
      onDragOverCapture={overModel}
      onDragLeave={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setDropping(false);
      }}
      onDropCapture={dropModel}
    >
      {dropping && (
        <div className="terminal-drop" aria-hidden>
          Drop to open the model in a new terminal
        </div>
      )}
      <header className="panel-header">
        <div className="panel-tabs" role="tablist">
          {tabs.map((tab) => {
            const title = tabTitle(tab, panes);
            const running = panes.get(tab.focused)?.running ?? false;
            return (
              // The close button is a sibling of the tab, not inside it (see EditorArea).
              <div key={tab.key} role="presentation" className={tab.key === active ? "panel-tab panel-tab-active" : "panel-tab"}>
                <button
                  type="button"
                  role="tab"
                  aria-selected={tab.key === active}
                  className={running ? "panel-tab-title" : "panel-tab-title panel-tab-ended"}
                  onClick={() => {
                    terminals.activate(tab.key);
                    if (collapsed) onShow();
                    terminals.requestFocus();
                  }}
                >
                  {title}
                </button>
                <button
                  type="button"
                  className="panel-tab-close"
                  aria-label={`Close ${title}`}
                  title="Close terminal (ends its processes)"
                  onClick={() => actions.closeTerminalTab(tab.key)}
                >
                  ×
                </button>
              </div>
            );
          })}
          <button type="button" className="icon-button" title="New Terminal (⌃⇧`)" aria-label="New terminal" onClick={() => actions.newTerminal()}>
            +
          </button>
        </div>
        {!collapsed && (
          <>
            <button
              type="button"
              className="icon-button"
              title="Split Right (⌘D in a terminal)"
              aria-label="Split terminal right"
              onClick={() => actions.splitTerminal("right")}
            >
              <SplitIcon direction="right" />
            </button>
            <button
              type="button"
              className="icon-button"
              title="Split Down (⇧⌘D in a terminal)"
              aria-label="Split terminal down"
              onClick={() => actions.splitTerminal("down")}
            >
              <SplitIcon direction="down" />
            </button>
          </>
        )}
        {collapsed ? (
          <button type="button" className="icon-button" title="Show Terminal (⌃`)" aria-label="Show terminal" onClick={onShow}>
            ⌃
          </button>
        ) : (
          <button type="button" className="icon-button" title="Hide Terminal (⌃`)" aria-label="Hide terminal" onClick={onHide}>
            ⌄
          </button>
        )}
      </header>
      {/* Collapsed, the terminals stay mounted and running, out of sight. */}
      <div className="panel-body" ref={body} hidden={collapsed}>
        {placed.map(({ key, rect, tab }) => {
          const split = paneKeys(tab.tree).length > 1;
          const focused = tab.focused === key;
          const title = panes.get(key)?.title ?? "Terminal";
          const classes = ["terminal-pane", split && "terminal-pane-split", split && focused && "terminal-pane-focused"];
          return (
            <div
              key={key}
              className={classes.filter(Boolean).join(" ")}
              style={rectStyle(rect)}
              hidden={tab.key !== active}
              role="group"
              aria-label={title}
              onFocus={() => terminals.focusPane(key)}
            >
              <PaneTerminal
                native={native}
                pane={panes.get(key)}
                visible={tab.key === active && !collapsed}
                focused={focused}
                focusRequest={focusRequest}
                terminals={terminals}
                font={font}
              />
              {split && (
                <button
                  type="button"
                  className="terminal-pane-close"
                  aria-label={`Close pane ${title}`}
                  title="Close pane (ends its processes)"
                  onClick={() => actions.closeTerminalPane(key)}
                >
                  ×
                </button>
              )}
            </div>
          );
        })}
        {activeTab &&
          layouts.get(activeTab.key)!.dividers.map((divider) => (
            <PaneDivider
              key={divider.id}
              container={body}
              {...divider}
              onResize={(ratio) => terminals.resize(activeTab.key, divider.id, ratio)}
            />
          ))}
        {tabs.length === 0 && (
          <div className="panel-empty">
            <button type="button" className="button-primary" onClick={() => actions.newTerminal()}>
              New Terminal
            </button>
          </div>
        )}
      </div>
    </section>
  );
}

/**
 * A pane's terminal. For an agent pane, the session is started with `agent_run`
 * for that agent session instead of `terminal_create`; everything after that is
 * the same. Memoized: the panel re-renders with every layout change (a splitter
 * drag), and an unchanged pane need not.
 */
const PaneTerminal = memo(function PaneTerminal({
  native,
  pane,
  visible,
  focused,
  focusRequest,
  terminals,
  font,
}: {
  native: PanelNative;
  pane: TerminalPane | undefined;
  visible: boolean;
  focused: boolean;
  focusRequest: number;
  terminals: Terminals;
  font: string | null;
}) {
  const kind: PaneKind = pane?.kind ?? { type: "shell" };
  const session = kind.type === "agent" ? kind.session : null;
  const install = kind.type === "install" ? kind.token : null;
  // Stable per pane: a new object would restart the session.
  const sessionNative = useMemo<SessionNative>(
    () =>
      session !== null
        ? { ...native, createTerminal: (size, listener) => native.runAgentSession(session, size, listener) }
        : install !== null
          ? { ...native, createTerminal: (size, listener) => native.installAddon(install, size, listener) }
          : native,
    [native, session, install],
  );
  if (!pane) return null;
  return (
    <TerminalView
      native={sessionNative}
      program={kind.type === "agent" ? kind.name : kind.type === "install" ? `the install of ${kind.name}` : undefined}
      once={kind.type === "install"}
      fontFamily={font ?? DEFAULT_FONT}
      visible={visible}
      focused={focused}
      focusRequest={focusRequest}
      onStart={(info) => terminals.started(pane.key, info)}
      onEnd={(ending) => terminals.ended(pane.key, ending)}
      onReader={(reader) => terminals.setReader(pane.key, reader)}
    />
  );
});

/** The draggable line between the two halves of a split. */
function PaneDivider({
  container,
  direction,
  ratio,
  area,
  onResize,
}: {
  container: RefObject<HTMLDivElement | null>;
  direction: SplitDirection;
  ratio: number;
  area: Rect;
  onResize: (ratio: number) => void;
}) {
  const horizontal = direction === "right";
  const style = horizontal
    ? { left: pct(area.x + area.width * ratio), top: pct(area.y), height: pct(area.height) }
    : { top: pct(area.y + area.height * ratio), left: pct(area.x), width: pct(area.width) };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    const box = container.current?.getBoundingClientRect();
    if (event.button !== 0 || !box) return;
    event.preventDefault();
    const bar = event.currentTarget;
    bar.setPointerCapture(event.pointerId);
    document.body.classList.add(horizontal ? "resizing-x" : "resizing-y");
    const move = (e: globalThis.PointerEvent) => {
      const at = horizontal ? (e.clientX - box.left) / box.width : (e.clientY - box.top) / box.height;
      onResize(horizontal ? (at - area.x) / area.width : (at - area.y) / area.height);
    };
    const stop = () => {
      bar.removeEventListener("pointermove", move);
      bar.removeEventListener("pointerup", stop);
      bar.removeEventListener("pointercancel", stop);
      document.body.classList.remove("resizing-x", "resizing-y");
    };
    bar.addEventListener("pointermove", move);
    bar.addEventListener("pointerup", stop);
    bar.addEventListener("pointercancel", stop);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const grow = horizontal ? "ArrowRight" : "ArrowDown";
    const shrink = horizontal ? "ArrowLeft" : "ArrowUp";
    if (event.key !== grow && event.key !== shrink) return;
    event.preventDefault();
    onResize(ratio + (event.key === grow ? KEY_STEP : -KEY_STEP));
  };

  return (
    <div
      className={`pane-divider pane-divider-${horizontal ? "x" : "y"}`}
      style={style}
      role="separator"
      aria-label="Resize terminal panes"
      aria-orientation={horizontal ? "vertical" : "horizontal"}
      aria-valuenow={Math.round(ratio * 100)}
      aria-valuemin={10}
      aria-valuemax={90}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onKeyDown={onKeyDown}
    />
  );
}

function SplitIcon({ direction }: { direction: SplitDirection }) {
  return (
    <svg width="14" height="14" viewBox="0 0 14 14" aria-hidden fill="none" stroke="currentColor" strokeWidth="1.2">
      <rect x="1.5" y="1.5" width="11" height="11" rx="1.5" />
      {direction === "right" ? <line x1="7" y1="1.5" x2="7" y2="12.5" /> : <line x1="1.5" y1="7" x2="12.5" y2="7" />}
    </svg>
  );
}

function rectStyle({ x, y, width, height }: Rect) {
  return { left: pct(x), top: pct(y), width: pct(width), height: pct(height) };
}

function pct(fraction: number): string {
  return `${fraction * 100}%`;
}
