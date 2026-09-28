import { useEffect, useState } from "react";

import type { AppInfo } from "../contracts/generated/AppInfo";
import { NativeError, type NativeClient } from "../native";

export type NativeStatus =
  | { state: "connecting" }
  | { state: "connected"; info: AppInfo }
  | { state: "failed"; error: NativeError };

/** Asks the native host who it is, once. A failure is surfaced, never swallowed. */
export function useNativeStatus(native: NativeClient): NativeStatus {
  const [status, setStatus] = useState<NativeStatus>({ state: "connecting" });

  useEffect(() => {
    let current = true;
    native.getAppInfo().then(
      (info) => {
        if (current) setStatus({ state: "connected", info });
      },
      (reason: unknown) => {
        const error = reason instanceof NativeError ? reason : NativeError.from("get_app_info", reason);
        console.error("Native host unavailable", error);
        if (current) setStatus({ state: "failed", error });
      },
    );
    return () => {
      current = false;
    };
  }, [native]);

  return status;
}
