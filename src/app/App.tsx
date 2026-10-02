import { useCallback, useEffect, useState } from "react";

import { ShareContextView } from "../agents/ShareContextView";
import { EditorArea } from "../editor/EditorArea";
import { HomeView } from "../home/HomeView";
import { useStore } from "../lib/useStore";
import type { NativeClient } from "../native";
import { TerminalPanel } from "../terminal/TerminalPanel";
import type { Workbench } from "../workbench/workbench";
import { DialogHost, NotificationList, PickerView } from "./Overlays";
import { Sidebar } from "./Sidebar";
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
  const { visible: homeVisible } = useStore(workbench.home);
  const windowHeight = useWindowHeight();
  useShortcuts(workbench);

  const terminalHeight = Math.min(layout.terminalHeight, windowHeight - MIN_EDITOR_HEIGHT);
  const openFile = useCallback((path: string) => workbench.openFile(path), [workbench]);
  const openRecent = useCallback((root: string) => workbench.openRecent(root), [workbench]);

  return (
    <div className="shell">
      {/* Under the welcome screen, the workspace keeps running but cannot take focus or clicks. */}
      <div
        className="workbench"
        inert={homeVisible}
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
              <Sidebar workbench={workbench} />
              <Splitter
                axis="x"
                label="Resize sidebar"
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
          actions={workbench}
          hidden={!layout.terminalVisible}
          onHide={() => workbench.layout.setTerminalVisible(false)}
        />
      </div>
      {homeVisible && <HomeView home={workbench.home} recent={workbench.recent} workspace={workbench.workspace} actions={workbench} />}
      <StatusBar
        status={status}
        workspace={workspace}
        onHome={() => workbench.showHome()}
        onTrust={(trusted) => void workbench.setTrust(trusted)}
      />
      <NotificationList notifications={workbench.notifications} />
      <PickerView picker={workbench.picker} onOpenFile={openFile} onOpenWorkspace={openRecent} />
      <ShareContextView share={workbench.share} agents={workbench.agents} actions={workbench} />
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
