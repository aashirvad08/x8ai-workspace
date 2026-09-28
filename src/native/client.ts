import type { AppInfo } from "../contracts/generated/AppInfo";
import { NativeError } from "./errors";

/** The shape of Tauri's `invoke`, injected so the client is testable without a webview. */
export type Invoke = (command: string, args?: Record<string, unknown>) => Promise<unknown>;

/**
 * Typed access to the native host. Each method maps to one command in
 * `src-tauri/src/commands.rs`. UI code depends on this interface, never on Tauri.
 */
export interface NativeClient {
  getAppInfo(): Promise<AppInfo>;
}

export function createNativeClient(invoke: Invoke): NativeClient {
  // Return values come from the trusted side and are typed by the generated
  // contracts, so they are cast rather than re-validated. Arguments going the other
  // way are validated in Rust.
  async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    try {
      return (await invoke(command, args)) as T;
    } catch (reason) {
      throw NativeError.from(command, reason);
    }
  }

  return {
    getAppInfo: () => call<AppInfo>("get_app_info"),
  };
}
