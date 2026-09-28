import type { AppEvent } from "../contracts/generated/AppEvent";
import type { AppInfo } from "../contracts/generated/AppInfo";
import type { DirEntry } from "../contracts/generated/DirEntry";
import type { FileContent } from "../contracts/generated/FileContent";
import type { FileList } from "../contracts/generated/FileList";
import type { FileVersion } from "../contracts/generated/FileVersion";
import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import type { SearchEvent } from "../contracts/generated/SearchEvent";
import type { SearchQuery } from "../contracts/generated/SearchQuery";
import type { SessionId } from "../contracts/generated/SessionId";
import type { TerminalEvent } from "../contracts/generated/TerminalEvent";
import type { TerminalInfo } from "../contracts/generated/TerminalInfo";
import type { TerminalSize } from "../contracts/generated/TerminalSize";
import type { WorkspaceEvent } from "../contracts/generated/WorkspaceEvent";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
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

export interface AppApi {
  getAppInfo(): Promise<AppInfo>;
  /** Registers for app-level events, such as a quit request with unsaved changes. */
  subscribeApp(listener: (event: AppEvent) => void): Promise<void>;
  /** Tells the native side whether quitting now would lose work. */
  setUnsavedChanges(unsaved: boolean): Promise<void>;
  /** Quits without asking again. */
  quit(): Promise<void>;
  /** Problems the native side found on its own, such as an unreadable settings file. Each is returned once. */
  takeWarnings(): Promise<string[]>;
}

export interface TerminalApi {
  /** Starts the user's login shell, in the workspace root if one is open. */
  createTerminal(size: TerminalSize, listener: TerminalListener): Promise<TerminalInfo>;
  writeTerminal(id: SessionId, data: Uint8Array): Promise<void>;
  resizeTerminal(id: SessionId, size: TerminalSize): Promise<void>;
  /** Flow control: `bytes` more of the session's output have been rendered. */
  ackTerminal(id: SessionId, bytes: number): Promise<void>;
  /** Whether a program other than the shell is running in the session, so closing it would end that program. */
  isTerminalBusy(id: SessionId): Promise<boolean>;
  /** Hangs up the session. */
  closeTerminal(id: SessionId): Promise<void>;
}

/**
 * File operations inside the open workspace. Paths are workspace paths: relative to
 * the root, `/`-separated, with `""` for the root itself.
 */
export interface WorkspaceApi {
  /**
   * Shows the native folder picker. The chosen folder becomes the workspace and
   * changes on disk are reported to `listener`. Resolves to `null` if cancelled.
   */
  openWorkspace(listener: (event: WorkspaceEvent) => void): Promise<WorkspaceInfo | null>;
  /**
   * Reopens a folder from the recent list, the only other way to open one. Fails
   * with `notFound` (and drops it from the list) if the folder is gone.
   */
  openRecentWorkspace(root: string, listener: (event: WorkspaceEvent) => void): Promise<WorkspaceInfo>;
  /** Recently opened folders, most recent first. */
  recentWorkspaces(): Promise<RecentWorkspace[]>;
  forgetRecentWorkspace(root: string): Promise<void>;
  /**
   * Trusts or stops trusting the open workspace. Trusting shows a native
   * confirmation; resolves to the workspace as it now is, unchanged if declined.
   */
  setWorkspaceTrust(trusted: boolean): Promise<WorkspaceInfo>;
  listDir(path: string): Promise<DirEntry[]>;
  readFile(path: string): Promise<FileContent>;
  /** `null` if the file does not exist. */
  fileVersion(path: string): Promise<FileVersion | null>;
  /**
   * Saves `text`. With `expected`, fails with code `conflict` if the file changed
   * or disappeared on disk since that version. With `null`, overwrites.
   */
  writeFile(path: string, text: string, expected: FileVersion | null): Promise<FileVersion>;
  createFile(path: string): Promise<void>;
  createDir(path: string): Promise<void>;
  renameEntry(from: string, to: string): Promise<void>;
  /** Moves the entry to the Trash. */
  deleteEntry(path: string): Promise<void>;
  /** Workspace files for quick open, gathered on demand. */
  listFiles(): Promise<FileList>;
  /**
   * Searches file contents for literal text. Each file's matches arrive on
   * `listener` as found, then a summary; resolves once the search is over. A new
   * search cancels the previous one.
   */
  search(query: SearchQuery, listener: (event: SearchEvent) => void): Promise<void>;
  cancelSearch(): Promise<void>;
}

/**
 * Typed access to the native host. Each method maps to one command in
 * `src-tauri/src/`. UI code depends on these interfaces, never on Tauri.
 */
export interface NativeClient extends AppApi, TerminalApi, WorkspaceApi {}

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
    subscribeApp: (listener) =>
      call<void>("app_subscribe", { events: createChannel((message) => listener(message as AppEvent)) }),
    setUnsavedChanges: (unsaved) => call<void>("app_set_unsaved_changes", { unsaved }),
    quit: () => call<void>("app_quit"),
    takeWarnings: () => call<string[]>("app_take_warnings"),

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
    isTerminalBusy: (id) => call<boolean>("terminal_is_busy", { id }),
    closeTerminal: (id) => call<void>("terminal_close", { id }),

    openWorkspace: (listener) =>
      call<WorkspaceInfo | null>("workspace_open", {
        events: createChannel((message) => listener(message as WorkspaceEvent)),
      }),
    openRecentWorkspace: (root, listener) =>
      call<WorkspaceInfo>("workspace_open_recent", {
        root,
        events: createChannel((message) => listener(message as WorkspaceEvent)),
      }),
    recentWorkspaces: () => call<RecentWorkspace[]>("workspace_recent"),
    forgetRecentWorkspace: (root) => call<void>("workspace_forget_recent", { root }),
    setWorkspaceTrust: (trusted) => call<WorkspaceInfo>("workspace_set_trust", { trusted }),
    listDir: (path) => call<DirEntry[]>("workspace_list_dir", { path }),
    readFile: (path) => call<FileContent>("workspace_read_file", { path }),
    fileVersion: (path) => call<FileVersion | null>("workspace_file_version", { path }),
    writeFile: (path, text, expected) => call<FileVersion>("workspace_write_file", { path, text, expected }),
    createFile: (path) => call<void>("workspace_create_file", { path }),
    createDir: (path) => call<void>("workspace_create_dir", { path }),
    renameEntry: (from, to) => call<void>("workspace_rename", { from, to }),
    deleteEntry: (path) => call<void>("workspace_delete", { path }),
    listFiles: () => call<FileList>("workspace_list_files"),
    search: (query, listener) =>
      call<void>("workspace_search", {
        query,
        events: createChannel((message) => listener(message as SearchEvent)),
      }),
    cancelSearch: () => call<void>("workspace_search_cancel"),
  };
}
