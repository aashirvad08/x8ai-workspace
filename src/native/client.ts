import type { AddonAddResult } from "../contracts/generated/AddonAddResult";
import type { AddonList } from "../contracts/generated/AddonList";
import type { AgentChanges } from "../contracts/generated/AgentChanges";
import type { AgentList } from "../contracts/generated/AgentList";
import type { AgentRemoval } from "../contracts/generated/AgentRemoval";
import type { AgentSessionId } from "../contracts/generated/AgentSessionId";
import type { AgentSessionInfo } from "../contracts/generated/AgentSessionInfo";
import type { AppEvent } from "../contracts/generated/AppEvent";
import type { AppInfo } from "../contracts/generated/AppInfo";
import type { CatalogList } from "../contracts/generated/CatalogList";
import type { DirEntry } from "../contracts/generated/DirEntry";
import type { FileContent } from "../contracts/generated/FileContent";
import type { FileList } from "../contracts/generated/FileList";
import type { FileVersion } from "../contracts/generated/FileVersion";
import type { McpServerInput } from "../contracts/generated/McpServerInput";
import type { McpServerList } from "../contracts/generated/McpServerList";
import type { McpServerStatus } from "../contracts/generated/McpServerStatus";
import type { ModelSelection } from "../contracts/generated/ModelSelection";
import type { ProviderList } from "../contracts/generated/ProviderList";
import type { ProviderStatus } from "../contracts/generated/ProviderStatus";
import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import type { SearchEvent } from "../contracts/generated/SearchEvent";
import type { SearchQuery } from "../contracts/generated/SearchQuery";
import type { SessionId } from "../contracts/generated/SessionId";
import type { Skill } from "../contracts/generated/Skill";
import type { SkillInput } from "../contracts/generated/SkillInput";
import type { SkillList } from "../contracts/generated/SkillList";
import type { SpaceInfo } from "../contracts/generated/SpaceInfo";
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
   * Shows the native folder picker, starting at `start` if given (`~` is the home
   * folder). The chosen folder becomes the workspace and changes on disk are
   * reported to `listener`. Resolves to `null` if cancelled.
   */
  openWorkspace(listener: (event: WorkspaceEvent) => void, start?: string | null): Promise<WorkspaceInfo | null>;
  /**
   * Makes the new, empty folder `~/Workspaces/<name>` and opens it as a new
   * space (`/new <name>`). Fails with `conflict` if it exists.
   */
  createWorkspace(name: string, listener: (event: WorkspaceEvent) => void): Promise<WorkspaceInfo>;
  /** Closes the open folder; its agents stop, and new terminals start in the home folder. */
  closeWorkspace(): Promise<void>;
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
 * External coding agents in the open workspace (docs/agent-runtime.md). The
 * webview names an agent by id; the native side decides what runs, where, and
 * whether it may.
 */
export interface AgentApi {
  /** Every built-in agent: installed or not, approved for the open workspace or not. */
  listAgents(refresh: boolean): Promise<AgentList>;
  /**
   * Makes sure the agent may run in the open workspace, with its own model
   * configuration (`model` null) or pointed at `model`, and with the MCP servers a
   * new session gets (the session-scoped servers `mcp` among them), asking the user
   * in one native dialog for what is not approved there yet. The session-scoped
   * skills `skills` are named in it; skills need no approval. Fails with
   * `permissionDenied` if the workspace is not trusted. Resolves to `false` if the
   * user declined.
   */
  requestAgentApproval(
    agent: string,
    model: ModelSelection | null,
    mcp: readonly string[],
    skills: readonly string[],
  ): Promise<boolean>;
  /** The same for an existing session, before it runs again. */
  requestSessionApproval(session: AgentSessionId): Promise<boolean>;
  /** Forgets the agent's approval in the open workspace. */
  revokeAgentApproval(agent: string): Promise<void>;
  /**
   * A new session for an approved agent in the open workspace (docs/multi-agent.md):
   * a worktree of its own in a Git repository, the folder itself otherwise. The
   * native side decides where; nothing is started yet. The session keeps `model`
   * (null: the agent's own configuration), its MCP servers and its skills (exactly
   * as they are now) for every run.
   */
  createAgentSession(
    agent: string,
    model: ModelSelection | null,
    mcp: readonly string[],
    skills: readonly string[],
  ): Promise<AgentSessionInfo>;
  /**
   * Runs the session's agent (again) on a new terminal session, driven afterwards
   * like any other with the `TerminalApi` methods.
   */
  runAgentSession(session: AgentSessionId, size: TerminalSize, listener: TerminalListener): Promise<TerminalInfo>;
  /** The open workspace's agent sessions, including worktrees from earlier runs. */
  agentSessions(): Promise<AgentSessionInfo[]>;
  stopAgentSession(session: AgentSessionId): Promise<void>;
  /** Removes a stopped session and its worktree; `discard` allows losing uncommitted changes. */
  removeAgentSession(session: AgentSessionId, discard: boolean): Promise<AgentRemoval>;
  /** What the agent changed in its worktree. */
  agentChanges(session: AgentSessionId): Promise<AgentChanges>;
  /** A file in the agent's worktree, for inspection. */
  readAgentFile(session: AgentSessionId, path: string): Promise<FileContent>;
}

/**
 * Model providers and their models (docs/models.md). A key goes to the native side
 * once, when the user saves it, and is never returned: the webview only learns
 * whether one is saved.
 */
export interface ProviderApi {
  /** Every provider; with `checkLocal`, looks for Ollama on this machine first. */
  listProviders(checkLocal: boolean): Promise<ProviderList>;
  /** Saves the provider's API key in the macOS Keychain. */
  setProviderCredential(provider: string, key: string): Promise<ProviderStatus>;
  removeProviderCredential(provider: string): Promise<ProviderStatus>;
  /** Adds a model id the provider serves that the app does not list. */
  addProviderModel(provider: string, model: string): Promise<ProviderStatus>;
  removeProviderModel(provider: string, model: string): Promise<ProviderStatus>;
}

/**
 * MCP servers the user configured (docs/mcp.md). A secret goes to the native side
 * once, to be saved, and is never returned. Nothing here starts or contacts a
 * server: agent sessions do, after trust and approval.
 */
export interface McpApi {
  listMcpServers(): Promise<McpServerList>;
  addMcpServer(server: McpServerInput): Promise<McpServerStatus>;
  updateMcpServer(id: string, server: McpServerInput): Promise<McpServerStatus>;
  setMcpServerEnabled(id: string, enabled: boolean): Promise<McpServerStatus>;
  removeMcpServer(id: string): Promise<void>;
  /** Saves the value of one of the server's secret variables in the macOS Keychain. */
  setMcpSecret(id: string, name: string, value: string): Promise<McpServerStatus>;
  removeMcpSecret(id: string, name: string): Promise<McpServerStatus>;
}

/** Skills (docs/catalog.md): instructions for agent sessions. Text only. */
export interface SkillApi {
  listSkills(): Promise<SkillList>;
  addSkill(skill: SkillInput): Promise<Skill>;
  updateSkill(id: string, skill: SkillInput): Promise<Skill>;
  removeSkill(id: string): Promise<void>;
}

/**
 * The catalog (docs/catalog.md): read-only discovery over agents, models, MCP
 * servers and skills. Acting on an item uses the owning system's own methods.
 */
export interface CatalogApi {
  listCatalog(): Promise<CatalogList>;
}

/**
 * Add-ons and spaces (docs/decisions/0019-add-ons.md): tools a space's terminals
 * use. The webview names add-ons and spaces by id; what an add-on installs and
 * runs is the native side's.
 */
export interface AddonApi {
  /** Every add-on, in the open space. `refresh` reads the login environment again. */
  listAddons(refresh: boolean): Promise<AddonList>;
  /**
   * Adds an add-on (and what it requires) to the open space. If something must
   * be installed first, a native dialog shows the commands; once confirmed, the
   * result has a token to run them with `installAddon`.
   */
  addAddon(id: string): Promise<AddonAddResult>;
  /** Runs a confirmed install, once, in a terminal; the add-on is added when it ends well. */
  installAddon(token: number, size: TerminalSize, listener: TerminalListener): Promise<TerminalInfo>;
  /** Takes the add-on out of the open space; nothing is uninstalled. */
  removeAddon(id: string): Promise<AddonList>;
  /** Every space with an id, the one with no folder first. */
  listSpaces(): Promise<SpaceInfo[]>;
  /** Adds add-ons of the open space to another space (`/share`). */
  shareSpace(to: string, addons: string[]): Promise<SpaceInfo>;
}

/**
 * Typed access to the native host. Each method maps to one command in
 * `src-tauri/src/`. UI code depends on these interfaces, never on Tauri.
 */
export interface NativeClient
  extends AppApi,
    TerminalApi,
    WorkspaceApi,
    AgentApi,
    ProviderApi,
    McpApi,
    SkillApi,
    CatalogApi,
    AddonApi {}

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

    createTerminal: (size, listener) =>
      call<TerminalInfo>("terminal_create", { size, events: sessionChannel(listener) }),
    // Raw body: bytes reach the PTY exactly as given, without JSON encoding.
    writeTerminal: (id, data) =>
      call<void>("terminal_write", data, { headers: { [SESSION_ID_HEADER]: String(id) } }),
    resizeTerminal: (id, size) => call<void>("terminal_resize", { id, size }),
    ackTerminal: (id, bytes) => call<void>("terminal_ack", { id, bytes }),
    isTerminalBusy: (id) => call<boolean>("terminal_is_busy", { id }),
    closeTerminal: (id) => call<void>("terminal_close", { id }),

    openWorkspace: (listener, start = null) =>
      call<WorkspaceInfo | null>("workspace_open", {
        start,
        events: createChannel((message) => listener(message as WorkspaceEvent)),
      }),
    createWorkspace: (name, listener) =>
      call<WorkspaceInfo>("workspace_create", {
        name,
        events: createChannel((message) => listener(message as WorkspaceEvent)),
      }),
    closeWorkspace: () => call<void>("workspace_close"),
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

    listAgents: (refresh) => call<AgentList>("agent_list", { refresh }),
    requestAgentApproval: (agent, model, mcp, skills) =>
      call<boolean>("agent_request_approval", { agent, model, mcp, skills }),
    requestSessionApproval: (session) => call<boolean>("agent_request_session_approval", { session }),
    revokeAgentApproval: (agent) => call<void>("agent_revoke", { agent }),
    createAgentSession: (agent, model, mcp, skills) =>
      call<AgentSessionInfo>("agent_create_session", { agent, model, mcp, skills }),
    runAgentSession: (session, size, listener) =>
      call<TerminalInfo>("agent_run", { session, size, events: sessionChannel(listener) }),
    agentSessions: () => call<AgentSessionInfo[]>("agent_sessions"),
    stopAgentSession: (session) => call<void>("agent_stop", { session }),
    removeAgentSession: (session, discard) => call<AgentRemoval>("agent_remove", { session, discard }),
    agentChanges: (session) => call<AgentChanges>("agent_changes", { session }),
    readAgentFile: (session, path) => call<FileContent>("agent_read_file", { session, path }),

    listProviders: (checkLocal) => call<ProviderList>("provider_list", { checkLocal }),
    setProviderCredential: (provider, key) => call<ProviderStatus>("provider_set_credential", { provider, key }),
    removeProviderCredential: (provider) => call<ProviderStatus>("provider_remove_credential", { provider }),
    addProviderModel: (provider, model) => call<ProviderStatus>("provider_add_model", { provider, model }),
    removeProviderModel: (provider, model) => call<ProviderStatus>("provider_remove_model", { provider, model }),

    listMcpServers: () => call<McpServerList>("mcp_list"),
    addMcpServer: (server) => call<McpServerStatus>("mcp_add", { server }),
    updateMcpServer: (id, server) => call<McpServerStatus>("mcp_update", { id, server }),
    setMcpServerEnabled: (id, enabled) => call<McpServerStatus>("mcp_set_enabled", { id, enabled }),
    removeMcpServer: (id) => call<void>("mcp_remove", { id }),
    setMcpSecret: (id, name, value) => call<McpServerStatus>("mcp_set_secret", { id, name, value }),
    removeMcpSecret: (id, name) => call<McpServerStatus>("mcp_remove_secret", { id, name }),

    listSkills: () => call<SkillList>("skill_list"),
    addSkill: (skill) => call<Skill>("skill_add", { skill }),
    updateSkill: (id, skill) => call<Skill>("skill_update", { id, skill }),
    removeSkill: (id) => call<void>("skill_remove", { id }),

    listCatalog: () => call<CatalogList>("catalog_list"),

    listAddons: (refresh) => call<AddonList>("addon_list", { refresh }),
    addAddon: (id) => call<AddonAddResult>("addon_add", { id }),
    installAddon: (token, size, listener) =>
      call<TerminalInfo>("addon_install", { token, size, events: sessionChannel(listener) }),
    removeAddon: (id) => call<AddonList>("addon_remove", { id }),
    listSpaces: () => call<SpaceInfo[]>("space_list"),
    shareSpace: (to, addons) => call<SpaceInfo>("space_share", { to, addons }),
  };

  /**
   * One channel carries a session's output as raw bytes (an ArrayBuffer) and its
   * lifecycle events as JSON, in the order the native side sent them.
   */
  function sessionChannel(listener: TerminalListener): unknown {
    return createChannel((message) => {
      if (message instanceof ArrayBuffer) {
        listener.output(new Uint8Array(message));
      } else {
        listener.event(message as TerminalEvent);
      }
    });
  }
}
