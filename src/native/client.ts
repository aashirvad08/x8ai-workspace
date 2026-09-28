import type { AppInfo } from "../contracts/generated/AppInfo";
import type { SessionId } from "../contracts/generated/SessionId";
import type { TerminalEvent } from "../contracts/generated/TerminalEvent";
import type { TerminalInfo } from "../contracts/generated/TerminalInfo";
import type { TerminalSize } from "../contracts/generated/TerminalSize";
import { NativeError } from "./errors";

/** Must match `SESSION_ID_HEADER` in crates/core/src/terminal.rs. */
export const SESSION_ID_HEADER = "x8ai-session-id";

export type InvokeArgs = Record<string, unknown> | Uint8Array;
export interface InvokeOptions {
  headers: Record<string, string>;
}

/** The shape of Tauri's `invoke`. */
export type Invoke = (command: string, args?: InvokeArgs, options?: InvokeOptions) => Promise<unknown>;

/**
 * Creates a Tauri channel that delivers messages to `onMessage`. The result is opaque
 * to the client: it is only passed along as a command argument.
 */
export type CreateChannel = (onMessage: (message: unknown) => void) => unknown;

/** The Tauri primitives the client is built on, injected so it is testable without a webview. */
export interface NativeBridge {
  invoke: Invoke;
  createChannel: CreateChannel;
}

/** Receives a terminal session's output and lifecycle events, in order. */
export interface TerminalListener {
  output(data: Uint8Array): void;
  event(event: TerminalEvent): void;
}

/**
 * Typed access to the native host. Each method maps to one command in
 * `src-tauri/src/`. UI code depends on this interface, never on Tauri.
 */
export interface NativeClient {
  getAppInfo(): Promise<AppInfo>;
  /** Starts the user's login shell in a new terminal session. */
  createTerminal(size: TerminalSize, listener: TerminalListener): Promise<TerminalInfo>;
  writeTerminal(id: SessionId, data: Uint8Array): Promise<void>;
  resizeTerminal(id: SessionId, size: TerminalSize): Promise<void>;
  /** Flow control: `bytes` more of the session's output have been rendered. */
  ackTerminal(id: SessionId, bytes: number): Promise<void>;
  /** Hangs up the session. */
  closeTerminal(id: SessionId): Promise<void>;
}

export function createNativeClient({ invoke, createChannel }: NativeBridge): NativeClient {
  // Return values come from the trusted side and are typed by the generated
  // contracts, so they are cast rather than re-validated. Arguments going the other
  // way are validated in Rust.
  async function call<T>(command: string, args?: InvokeArgs, options?: InvokeOptions): Promise<T> {
    try {
      return (await invoke(command, args, options)) as T;
    } catch (reason) {
      throw NativeError.from(command, reason);
    }
  }

  return {
    getAppInfo: () => call<AppInfo>("get_app_info"),

    createTerminal: (size, listener) => {
      // One channel carries both: output as raw bytes (an ArrayBuffer) and
      // lifecycle events as JSON, in the order the native side sent them.
      const events = createChannel((message) => {
        if (message instanceof ArrayBuffer) {
          listener.output(new Uint8Array(message));
        } else {
          listener.event(message as TerminalEvent);
        }
      });
      return call<TerminalInfo>("terminal_create", { size, events });
    },

    // Raw body: bytes reach the PTY exactly as given, without JSON encoding.
    writeTerminal: (id, data) =>
      call<void>("terminal_write", data, { headers: { [SESSION_ID_HEADER]: String(id) } }),

    resizeTerminal: (id, size) => call<void>("terminal_resize", { id, size }),

    ackTerminal: (id, bytes) => call<void>("terminal_ack", { id, bytes }),

    closeTerminal: (id) => call<void>("terminal_close", { id }),
  };
}
