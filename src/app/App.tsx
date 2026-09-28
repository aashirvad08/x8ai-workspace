import type { NativeClient } from "../native";
import { TerminalView } from "../terminal/TerminalView";
import { StatusBar } from "./StatusBar";
import { useNativeStatus } from "./useNativeStatus";
import "./app.css";

/**
 * The workspace shell. The terminal is the primary surface and fills the window;
 * other regions arrive with their phases (docs/roadmap.md).
 */
export function App({ native }: { native: NativeClient }) {
  const status = useNativeStatus(native);

  return (
    <div className="shell">
      <main className="workspace">
        <TerminalView native={native} />
      </main>
      <StatusBar status={status} />
    </div>
  );
}
