import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import type { NativeStatus } from "./useNativeStatus";

export function StatusBar({ status, workspace }: { status: NativeStatus; workspace: WorkspaceInfo | null }) {
  return (
    <footer className={status.state === "failed" ? "statusbar statusbar-error" : "statusbar"}>
      <Connection status={status} />
      <span className="statusbar-spacer" />
      {workspace && <span title={workspace.root}>{workspace.root}</span>}
    </footer>
  );
}

function Connection({ status }: { status: NativeStatus }) {
  switch (status.state) {
    case "connecting":
      return (
        <span>
          <span className="dot" /> Connecting to native host…
        </span>
      );
    case "connected": {
      const { name, version, os, arch } = status.info;
      return (
        <span>
          <span className="dot dot-ok" /> Native host connected · {name} {version} · {os}/{arch}
        </span>
      );
    }
    case "failed":
      return (
        <span role="alert">
          <span className="dot dot-error" /> Native host unavailable ({status.error.code}): {status.error.message}
        </span>
      );
  }
}
