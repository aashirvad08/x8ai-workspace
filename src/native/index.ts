// The frontend's only gateway to the native host. Nothing outside src/native/ may
// import @tauri-apps/* (enforced by src/architecture.test.ts).
import { Channel, invoke } from "@tauri-apps/api/core";

import { createNativeClient, type NativeClient } from "./client";

export type { AgentApi, AppApi, McpApi, NativeClient, ProviderApi, TerminalApi, TerminalListener, WorkspaceApi } from "./client";
export { NativeError, type NativeErrorCode } from "./errors";

/** Binds the client to the real Tauri bridge. Called once, from the composition root. */
export function createTauriNativeClient(): NativeClient {
  return createNativeClient({
    invoke,
    createChannel: (onMessage) => new Channel<unknown>(onMessage),
  });
}
