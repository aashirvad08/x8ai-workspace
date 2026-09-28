import { useCallback, useEffect, useState } from "react";

import { EditorArea } from "../editor/EditorArea";
import { useStore } from "../lib/useStore";
import type { NativeClient } from "../native";
import { TerminalPanel } from "../terminal/TerminalPanel";
import type { Workbench } from "../workbench/workbench";
import { FileExplorer } from "../workspace/FileExplorer";
import { DialogHost, NotificationList, PickerView } from "./Overlays";
import { Splitter } from "./Splitter";
import { StatusBar } from "./StatusBar";
import { useNativeStatus } from "./useNativeStatus";
import { useShortcuts } from "./useShortcuts";
import "./app.css";

/** The editor area always keeps at least this much height. */
const MIN_EDITOR_HEIGHT = 160;

/**
 * The workspace window: files and editor above, terminal below. The terminal
 * spans the full width and stays one shortcut away (⌃`).
 */
export function App({ workbench, native }: { workbench: Workbench; native: NativeClient }) {
  const status = useNativeStatus(native);
  const layout = useStore(workbench.layout);
  const workspace = useStore(workbench.workspace);
  const windowHeight = useWindowHeight();
  useShortcuts(workbench);

  const terminalHeight = Math.min(layout.terminalHeight, windowHeight - MIN_EDITOR_HEIGHT);
  const openFile = useCallback((path: string) => workbench.openFile(path), [workbench]);

  return (
    <div className="shell">
      <div
        className="workbench"
        style={{
          gridTemplateRows: layout.terminalVisible ? `minmax(0, 1fr) auto ${terminalHeight}px` : "minmax(0, 1fr)",
        }}
      >
        <div
          className="workbench-top"
          style={{
            gridTemplateColumns: layout.explorerVisible ? `${layout.explorerWidth}px auto minmax(0, 1fr)` : "minmax(0, 1fr)",
          }}
        >
          {layout.explorerVisible && (
            <>
              <FileExplorer explorer={workbench.explorer} workspace={workbench.workspace} actions={workbench} />
              <Splitter
                axis="x"
                label="Resize file explorer"
                size={layout.explorerWidth}
                onResize={(width) => workbench.layout.resizeExplorer(width)}
              />
            </>
          )}
          <EditorArea editor={workbench.editor} actions={workbench} />
        </div>
        {layout.terminalVisible && (
          <Splitter
            axis="y"
            inverted
            label="Resize terminal"
            size={terminalHeight}
            onResize={(height) => workbench.layout.resizeTerminal(Math.min(height, windowHeight - MIN_EDITOR_HEIGHT))}
          />
        )}
        <TerminalPanel
          native={native}
          terminals={workbench.terminals}
          hidden={!layout.terminalVisible}
          onHide={() => workbench.layout.setTerminalVisible(false)}
        />
      </div>
      <StatusBar status={status} workspace={workspace} />
      <NotificationList notifications={workbench.notifications} />
      <PickerView picker={workbench.picker} onOpenFile={openFile} />
      <DialogHost dialogs={workbench.dialogs} />
    </div>
  );
}

function useWindowHeight(): number {
  const [height, setHeight] = useState(() => window.innerHeight);
  useEffect(() => {
    const onResize = () => setHeight(window.innerHeight);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return height;
}
