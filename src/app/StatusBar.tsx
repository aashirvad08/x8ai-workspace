import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import type { NativeStatus } from "./useNativeStatus";

interface Props {
  status: NativeStatus;
  workspace: WorkspaceInfo | null;
  onHome: () => void;
  onTrust: (trusted: boolean) => void;
}

export function StatusBar({ status, workspace, onHome, onTrust }: Props) {
  return (
    <footer className={status.state === "failed" ? "statusbar statusbar-error" : "statusbar"}>
      <button type="button" className="statusbar-home" title="Welcome (⇧⌘H)" onClick={onHome}>
        ⌂ Welcome
      </button>
      <Connection status={status} />
      <span className="statusbar-spacer" />
      {workspace && (
        <>
          <button
            type="button"
            className={workspace.trusted ? "statusbar-trust statusbar-trusted" : "statusbar-trust"}
            title={
              workspace.trusted
                ? "You trusted this folder. Click to remove trust."
                : "This folder is not trusted. Nothing runs in it automatically. Click to trust it."
            }
            onClick={() => onTrust(!workspace.trusted)}
          >
            {workspace.trusted ? "Trusted" : "Untrusted"}
          </button>
          <span title={workspace.root}>{workspace.root}</span>
        </>
      )}
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
