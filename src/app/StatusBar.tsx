import type { NativeStatus } from "./useNativeStatus";

export function StatusBar({ status }: { status: NativeStatus }) {
  switch (status.state) {
    case "connecting":
      return (
        <footer className="statusbar">
          <span className="dot" /> Connecting to native host…
        </footer>
      );
    case "connected": {
      const { name, version, os, arch } = status.info;
      return (
        <footer className="statusbar">
          <span className="dot dot-ok" /> Native host connected · {name} {version} · {os}/{arch}
        </footer>
      );
    }
    case "failed":
      return (
        <footer className="statusbar statusbar-error" role="alert">
          <span className="dot dot-error" /> Native host unavailable ({status.error.code}): {status.error.message}
        </footer>
      );
  }
}
