import type { AgentActions, ContextActions } from "../agents/actions";
import { Agents } from "../agents/agents";
import { composeContext, MAX_OUTPUT_LINES, matchSessions, pasteable, sessionWhere } from "../agents/context";
import { ContextShare, DEFAULT_PARTS, type ShareMode, type ShareParts } from "../agents/share";
import { LaunchDrafts } from "../agents/draft";
import type { CatalogActions } from "../catalog/actions";
import { Catalog } from "../catalog/catalog";
import type { AgentChanges } from "../contracts/generated/AgentChanges";
import type { AgentSessionInfo } from "../contracts/generated/AgentSessionInfo";
import type { AgentStatus } from "../contracts/generated/AgentStatus";
import type { DirEntry } from "../contracts/generated/DirEntry";
import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import type { SearchMatch } from "../contracts/generated/SearchMatch";
import type { WorkspaceEvent } from "../contracts/generated/WorkspaceEvent";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import type { McpServerInput } from "../contracts/generated/McpServerInput";
import type { SkillInput } from "../contracts/generated/SkillInput";
import type { ModelSelection } from "../contracts/generated/ModelSelection";
import type { EditorActions } from "../editor/actions";
import type { HomeActions } from "../home/actions";
import { Home, MAX_NAME_LENGTH, matchRecent, parseCommand } from "../home/home";
import { EditorStore } from "../editor/editor-store";
import { basename, dirname, join } from "../lib/paths";
import { Value } from "../lib/store";
import type { McpActions } from "../mcp/actions";
import { McpServers, mcpChoices } from "../mcp/servers";
import type { ModelActions } from "../models/actions";
import { agentForModel, modelChoices, Providers } from "../models/providers";
import { Skills, skillChoices } from "../skills/skills";
import { type NativeClient, NativeError } from "../native";
import type { TerminalActions } from "../terminal/actions";
import type { SplitDirection } from "../terminal/panes";
import { type TerminalPane, type TerminalReader, Terminals } from "../terminal/terminals";
import type { ExplorerActions, SearchActions } from "../workspace/actions";
import { Explorer } from "../workspace/explorer";
import { Search } from "../workspace/search";
import type { Command } from "./commands";
import { Dialogs } from "./dialogs";
import { Layout } from "./layout";
import { messageOf, Notifications } from "./notifications";
import { Picker } from "./picker";

/** How long a starting agent may take to get ready for the context. */
const PASTE_READY_MS = 45_000;
/** How long its terminal must be quiet: it has drawn its prompt. */
const QUIET_MS = 600;

/**
 * The workspace as the user works with it: the open folder, its file tree, editor
 * tabs, terminals, search, and everything that coordinates them. Every user
 * action lives here, so components stay presentational and the behaviour is
 * testable without React.
 */
export class Workbench
  implements
    ExplorerActions,
    EditorActions,
    TerminalActions,
    SearchActions,
    AgentActions,
    ModelActions,
    McpActions,
    CatalogActions,
    HomeActions,
    ContextActions
{
  readonly workspace = new Value<WorkspaceInfo | null>(null);
  /** Recently opened folders, most recent first. */
  readonly recent = new Value<readonly RecentWorkspace[]>([]);
  readonly explorer: Explorer;
  readonly editor: EditorStore;
  readonly search: Search;
  readonly agents: Agents;
  readonly providers: Providers;
  readonly mcp: McpServers;
  readonly skills: Skills;
  readonly catalog: Catalog;
  /** The welcome screen, the app's head. */
  readonly home: Home;
  /** The context composer: handing context from agent sessions to another. */
  readonly share = new ContextShare();
  /** Each source's changes while the composer is open: read once, or why not. */
  readonly #shareChanges = new Map<number, AgentChanges | string>();
  /** What the next launch of each agent asks for, from its card or the catalog. */
  readonly drafts = new LaunchDrafts();
  readonly terminals = new Terminals();
  readonly notifications = new Notifications();
  readonly dialogs = new Dialogs();
  readonly layout = new Layout();
  readonly picker = new Picker();

  readonly #native: NativeClient;
  /** Identifies the shown workspace's disk events (see `#open`). */
  #shown: symbol | null = null;
  /** Which agent panes run, as last seen, to refresh sessions when that changes. */
  #agentPanesSeen = "";
  #files: { paths: readonly string[]; truncated: boolean } | null = null;
  #reportedUnsaved = false;

  constructor(native: NativeClient) {
    this.#native = native;
    this.explorer = new Explorer(native);
    this.editor = new EditorStore(native);
    this.search = new Search(native);
    this.agents = new Agents(native);
    this.providers = new Providers(native);
    this.mcp = new McpServers(native);
    this.skills = new Skills(native);
    this.catalog = new Catalog(native);
    this.home = new Home(native);
    this.editor.subscribe(() => this.#reportUnsaved());
    this.terminals.subscribe(() => this.#agentPanesChanged());
  }

  /**
   * Connects to the native host: quit requests and startup warnings. Reopens the
   * most recent workspace if it still exists, behind the welcome screen, then
   * starts a first terminal (in that workspace, if one opened).
   */
  async start(): Promise<void> {
    this.#native.subscribeApp((event) => {
      if (event.type === "quitRequested") void this.#quitRequested();
    }).catch((error: unknown) => this.notifications.error(`Could not connect to the app: ${messageOf(error)}`));
    void this.home.load();
    await this.#refreshRecent();
    const last = this.recent.get()[0];
    if (last?.available) await this.#open((listener) => this.#native.openRecentWorkspace(last.root, listener), true);
    if (this.terminals.get().tabs.length === 0) this.terminals.add();
    await this.#showWarnings();
  }

  // Workspace

  /** The native folder picker, starting at `start` if given. Resolves to whether a folder opened. */
  async openFolder(start: string | null = null): Promise<boolean> {
    if (!(await this.#confirmStopAgents())) return false;
    if (!(await this.#confirmDiscard("Opening another folder closes its open files."))) return false;
    return this.#open((listener) => this.#native.openWorkspace(listener, start));
  }

  openRecent(root: string): void {
    void this.#openRecent(root);
  }

  async #openRecent(root: string): Promise<boolean> {
    if (this.workspace.get()?.root === root) return true;
    if (!(await this.#confirmStopAgents())) return false;
    if (!(await this.#confirmDiscard("Opening another folder closes its open files."))) return false;
    return this.#open((listener) => this.#native.openRecentWorkspace(root, listener));
  }

  /**
   * Closes the open folder, after asking about running agents and unsaved files:
   * the workspace with no folder, a new shell in the home folder. Resolves to
   * whether it closed (or none was open).
   */
  async closeFolder(): Promise<boolean> {
    if (!this.workspace.get()) return true;
    if (!(await this.#confirmStopAgents("Agents run only in the folder they were allowed in. Closing it stops them."))) return false;
    if (!(await this.#confirmDiscard("Closing the folder closes its open files."))) return false;
    try {
      await this.#native.closeWorkspace();
    } catch (error) {
      this.notifications.error(`Could not close the folder: ${messageOf(error)}`);
      return false;
    }
    this.#shown = null;
    this.editor.closeAll();
    this.#files = null;
    this.workspace.set(null);
    this.explorer.reset(false);
    this.search.reset();
    // The folder's agents stopped natively; shells stay, and a new one starts at home.
    this.terminals.closeAgents();
    this.terminals.add();
    this.#reloadAgents();
    await this.#refreshRecent();
    return true;
  }

  // Welcome (the head)

  /** ⇧⌘H: the welcome screen over the workspace, which keeps running. */
  showHome(): void {
    this.home.show();
  }

  leaveHome(): void {
    this.home.hide();
    this.terminals.requestFocus();
  }

  /**
   * `/cd <folder>`: a recent space opens at once; any other folder in the native
   * picker, starting there (the user chooses it; the interface cannot open a
   * folder by itself). `/home`: the workspace with no folder. `/name`: who the
   * welcome greets. The welcome screen closes once a space opens.
   */
  async runHomeCommand(text: string): Promise<void> {
    const command = parseCommand(text);
    if (command.kind === "empty") {
      this.leaveHome();
      return;
    }
    if (command.kind === "unknown") {
      this.home.say(`“${command.word}” is not a command here. Try /cd <folder>, /home, /get, /give or /name <your name>.`, "error");
      return;
    }
    switch (command.name) {
      case "/home":
        if (await this.closeFolder()) this.leaveHome();
        return;
      case "/get":
      case "/give": {
        let session: number | null = null;
        if (command.arg) {
          await this.agents.loadSessions();
          const found = matchSessions(command.arg, this.agents.get().sessions);
          const running = found.filter((s) => this.terminals.paneOfSession(s.id)?.running);
          const pick = found.length === 1 ? found[0] : running.length === 1 ? running[0] : undefined;
          if (!pick) {
            this.home.say(
              found.length === 0
                ? `No agent session matches “${command.arg}”.`
                : `Several sessions match “${command.arg}”; open the composer with ${command.name} and choose.`,
              "error",
            );
            return;
          }
          session = pick.id;
        }
        this.leaveHome();
        await this.#shareContext(command.name === "/get" ? "get" : "give", session);
        return;
      }
      case "/name": {
        const name = command.arg.slice(0, MAX_NAME_LENGTH);
        this.home.setName(name || null);
        this.home.say(name ? `Hello, ${name}.` : "Greeting you with your account's name again.");
        return;
      }
      case "/cd": {
        if (command.arg === "") {
          if (!(await this.openFolder())) this.home.say("No folder was opened.");
          return;
        }
        const matches = matchRecent(command.arg, this.recent.get());
        if (matches.length > 1) {
          this.home.say(`Several spaces match “${command.arg}”: ${matches.map((m) => m.root).join(", ")}. Type more of the path.`, "error");
          return;
        }
        const [match] = matches;
        if (match) {
          if (match.root === this.workspace.get()?.root) this.leaveHome();
          else if (!(await this.#openRecent(match.root))) this.home.say(`${match.name} was not opened.`);
          return;
        }
        if (!(await this.openFolder(command.arg))) {
          this.home.say(`${command.arg} is not one of your spaces yet. Choose it in the folder picker to open it.`);
        }
        return;
      }
    }
  }

  forgetRecent(root: string): void {
    this.#native
      .forgetRecentWorkspace(root)
      .catch((error: unknown) => this.notifications.error(`Could not remove it from Recent: ${messageOf(error)}`))
      .finally(() => void this.#refreshRecent());
  }

  /** ⌃R: the recent list in the picker. */
  async showRecent(): Promise<void> {
    await this.#refreshRecent();
    const others = this.recent.get().filter((w) => w.root !== this.workspace.get()?.root);
    if (others.length === 0) {
      this.notifications.info("No other recent folders. Open one with ⌘O.");
      return;
    }
    this.picker.showWorkspaces(others);
  }

  /**
   * Trusts the open workspace (the native side asks the user to confirm) or,
   * after confirming here, stops trusting it.
   */
  async setTrust(trusted: boolean): Promise<void> {
    const current = this.workspace.get();
    if (!current || current.trusted === trusted) return;
    if (!trusted) {
      const choice = await this.dialogs.ask({
        title: `Stop trusting “${current.name}”?`,
        message:
          "The folder goes back to untrusted. Nothing in it is changed. Agents lose their approval for it, and agents running in it stop.",
        buttons: [
          { label: "Remove Trust", value: "untrust", role: "primary" },
          { label: "Cancel", value: "cancel" },
        ],
        cancel: "cancel",
      });
      if (choice !== "untrust") return;
    }
    try {
      const info = await this.#native.setWorkspaceTrust(trusted);
      // Only if the same workspace is still open.
      if (this.workspace.get()?.root !== info.root) return;
      this.workspace.set(info);
      // The native side stopped the folder's agents when trust went.
      if (!info.trusted) this.terminals.closeAgents();
      this.#reloadAgents();
    } catch (error) {
      this.notifications.error(`Could not change trust: ${messageOf(error)}`);
    }
  }

  /**
   * Opens a workspace and shows it. `null` (a cancelled picker) keeps the current
   * one. Disk events are followed only for the workspace that is shown: events
   * still arriving from a previous one are ignored.
   */
  /** Resolves to whether a folder opened. The welcome screen closes then, unless `keepHome`. */
  async #open(
    opening: (listener: (event: WorkspaceEvent) => void) => Promise<WorkspaceInfo | null>,
    keepHome = false,
  ): Promise<boolean> {
    const token = Symbol("workspace");
    let info: WorkspaceInfo | null;
    try {
      info = await opening((event) => {
        if (this.#shown === token) void this.#diskChanged(event);
      });
    } catch (error) {
      this.notifications.error(`Could not open the folder: ${messageOf(error)}`);
      // A folder that is gone has been dropped from the list.
      await this.#refreshRecent();
      return false;
    } finally {
      await this.#showWarnings();
    }
    if (!info) return false;
    this.#shown = token;
    if (!keepHome) this.home.hide();
    this.editor.closeAll();
    this.#files = null;
    this.workspace.set(info);
    this.explorer.reset(true);
    this.search.reset();
    // Agents belong to the folder they were allowed in; the native side stopped
    // them. Shells stay where they are, and a new one starts in the new folder.
    this.terminals.closeAgents();
    this.terminals.add();
    this.#reloadAgents();
    await this.#refreshRecent();
    return true;
  }

  async #refreshRecent(): Promise<void> {
    try {
      this.recent.set(await this.#native.recentWorkspaces());
    } catch (error) {
      this.notifications.error(`Could not read recent folders: ${messageOf(error)}`);
    }
  }

  async #showWarnings(): Promise<void> {
    try {
      for (const warning of await this.#native.takeWarnings()) this.notifications.error(warning);
    } catch {
      // Warnings are best effort; the connection error is reported elsewhere.
    }
  }

  async #diskChanged(event: WorkspaceEvent): Promise<void> {
    this.#files = null;
    const paths = event.type === "rescan" ? "all" : event.paths;
    await Promise.all([this.explorer.diskChanged(paths), this.editor.diskChanged(paths)]);
  }

  // Explorer

  openFile(path: string): void {
    this.editor.open(path).catch((error: unknown) => {
      this.notifications.error(`Could not open ${path}: ${messageOf(error)}`);
    });
  }

  // Search

  openMatch(path: string, match: SearchMatch): void {
    this.editor
      .open(path)
      .then(() => this.editor.select(path, match.line, match.column, match.length))
      .catch((error: unknown) => this.notifications.error(`Could not open ${path}: ${messageOf(error)}`));
  }

  /** ⇧⌘F: the search view, with its field focused. */
  showSearch(): void {
    this.layout.showSidebar("search");
    this.search.requestFocus();
  }

  async create(parent: string, name: string, kind: "file" | "folder"): Promise<boolean> {
    const path = join(parent, name);
    try {
      await (kind === "file" ? this.#native.createFile(path) : this.#native.createDir(path));
    } catch (error) {
      this.notifications.error(`Could not create ${name}: ${messageOf(error)}`);
      return false;
    }
    await this.explorer.load(parent);
    this.explorer.select(path);
    if (kind === "file") this.openFile(path);
    return true;
  }

  async rename(path: string, name: string): Promise<boolean> {
    const to = join(dirname(path), name);
    if (to === path) return true;
    try {
      await this.#native.renameEntry(path, to);
    } catch (error) {
      this.notifications.error(`Could not rename ${basename(path)}: ${messageOf(error)}`);
      return false;
    }
    this.editor.renamed(path, to);
    this.explorer.renamed(path, to);
    await this.explorer.load(dirname(path));
    return true;
  }

  remove(entry: DirEntry): void {
    void this.#remove(entry);
  }

  async #remove(entry: DirEntry): Promise<void> {
    const what = entry.kind === "directory" ? `the folder "${entry.name}" and everything in it` : `"${entry.name}"`;
    const choice = await this.dialogs.ask({
      title: `Move ${what} to the Trash?`,
      message: "You can restore it from the Trash in Finder.",
      buttons: [
        { label: "Move to Trash", value: "delete", role: "destructive" },
        { label: "Cancel", value: "cancel" },
      ],
      cancel: "cancel",
    });
    if (choice !== "delete") return;
    try {
      await this.#native.deleteEntry(entry.path);
    } catch (error) {
      this.notifications.error(`Could not delete ${entry.name}: ${messageOf(error)}`);
      return;
    }
    this.explorer.removed(entry.path);
    // Open tabs for deleted files stay, marked as deleted, so no edits are lost.
    await Promise.all([this.explorer.load(dirname(entry.path)), this.editor.diskChanged([entry.path])]);
  }

  startCreating(kind: "file" | "folder"): void {
    if (!this.workspace.get()) return;
    const { selected, listings } = this.explorer.get();
    const entry = selected === null ? undefined : listings.get(dirname(selected))?.entries?.find((e) => e.path === selected);
    // Inside the selected folder, or next to the selected file.
    const parent = !entry ? "" : entry.kind === "directory" ? entry.path : dirname(entry.path);
    this.explorer.startEditing({ kind: kind === "file" ? "newFile" : "newFolder", parent });
  }

  // Editor

  save(path: string): void {
    void this.#save(path);
  }

  overwrite(path: string): void {
    void this.#save(path, true);
  }

  revert(path: string): void {
    this.editor.reload(path).catch((error: unknown) => {
      this.notifications.error(`Could not reload ${basename(path)}: ${messageOf(error)}`);
    });
  }

  close(path: string): void {
    void this.closeEditor(path);
  }

  /** Resolves to whether the tab was closed. Asks before discarding unsaved changes. */
  async closeEditor(path: string): Promise<boolean> {
    const tab = this.editor.get().tabs.find((t) => t.path === path);
    if (!tab) return true;
    if (tab.dirty) {
      const choice = await this.dialogs.ask({
        title: `Save changes to "${tab.name}"?`,
        message: "Your changes will be lost if you don't save them.",
        buttons: [
          { label: "Save", value: "save", role: "primary" },
          { label: "Don't Save", value: "discard", role: "destructive" },
          { label: "Cancel", value: "cancel" },
        ],
        cancel: "cancel",
      });
      if (choice === "cancel") return false;
      if (choice === "save" && !(await this.#save(path))) return false;
    }
    this.editor.close(path);
    return true;
  }

  /** Resolves to whether every dirty tab was saved. */
  async saveAll(): Promise<boolean> {
    const results = await Promise.all(this.editor.dirtyPaths().map((path) => this.#save(path)));
    return results.every(Boolean);
  }

  /** Resolves to whether the file was saved. Failures and conflicts are shown. */
  async #save(path: string, overwrite = false): Promise<boolean> {
    try {
      await this.editor.save(path, { overwrite });
      return true;
    } catch (error) {
      const name = basename(path);
      if (error instanceof NativeError && error.code === "conflict") {
        const deleted = this.editor.get().tabs.find((t) => t.path === path)?.disk === "deleted";
        this.notifications.error(
          deleted
            ? `"${name}" was deleted on disk. Your version was not saved.`
            : `"${name}" changed on disk since you opened it. Your version was not saved.`,
          [
            { label: deleted ? "Save Anyway" : "Overwrite", run: () => this.overwrite(path) },
            ...(deleted ? [] : [{ label: "Revert to Disk", run: () => this.revert(path) }]),
          ],
        );
      } else {
        this.notifications.error(`Could not save "${name}": ${messageOf(error)}`);
      }
      return false;
    }
  }

  // Quick open and commands

  async quickOpen(): Promise<void> {
    if (!this.workspace.get()) {
      this.notifications.info("Open a folder first (⌘O).");
      return;
    }
    if (this.#files) {
      this.picker.showFiles(this.#files.paths, this.#files.truncated);
      return;
    }
    this.picker.showFiles(null);
    try {
      const list = await this.#native.listFiles();
      this.#files = list;
      if (this.picker.get()?.kind === "files") this.picker.showFiles(list.paths, list.truncated);
    } catch (error) {
      this.picker.close();
      this.notifications.error(`Could not list files: ${messageOf(error)}`);
    }
  }

  showCommands(): void {
    this.picker.showCommands(this.commands().filter((command) => command.id !== "commands.show"));
  }

  commands(): Command[] {
    const active = () => this.editor.get().active;
    const pane = () => this.terminals.activeTab()?.focused;
    return [
      { id: "workspace.open", title: "Open Folder…", shortcut: { key: "o", meta: true }, run: () => void this.openFolder() },
      { id: "workspace.openRecent", title: "Open Recent Folder…", shortcut: { key: "r", ctrl: true }, run: () => void this.showRecent() },
      { id: "workspace.close", title: "Close Folder", run: () => void this.closeFolder() },
      { id: "view.home", title: "Show Welcome", shortcut: { key: "h", meta: true, shift: true }, run: () => this.showHome() },
      { id: "context.get", title: "Get Context for an Agent…", run: () => this.shareContext("get") },
      { id: "context.give", title: "Give Context to Another Agent…", run: () => this.shareContext("give") },
      { id: "workspace.trust", title: "Trust This Folder…", run: () => void this.setTrust(true) },
      { id: "workspace.untrust", title: "Remove Trust from This Folder…", run: () => void this.setTrust(false) },
      { id: "file.quickOpen", title: "Go to File…", shortcut: { key: "p", meta: true }, run: () => void this.quickOpen() },
      { id: "commands.show", title: "Show All Commands", shortcut: { key: "p", meta: true, shift: true }, run: () => this.showCommands() },
      { id: "file.new", title: "New File…", shortcut: { key: "n", meta: true }, run: () => this.startCreating("file") },
      { id: "folder.new", title: "New Folder…", run: () => this.startCreating("folder") },
      { id: "file.save", title: "Save", shortcut: { key: "s", meta: true }, run: () => void withActive(active(), (p) => this.save(p)) },
      { id: "file.saveAll", title: "Save All", shortcut: { key: "s", meta: true, alt: true }, run: () => void this.saveAll() },
      { id: "editor.close", title: "Close Editor", shortcut: { key: "w", meta: true }, when: "terminalNotFocused", run: () => void withActive(active(), (p) => this.close(p)) },
      { id: "file.revert", title: "Revert File", run: () => void withActive(active(), (p) => this.revert(p)) },
      { id: "view.explorer", title: "Toggle Sidebar", shortcut: { key: "b", meta: true }, run: () => this.layout.toggleExplorer() },
      { id: "view.files", title: "Show File Explorer", shortcut: { key: "e", meta: true, shift: true }, run: () => this.layout.showSidebar("files") },
      { id: "search.show", title: "Search in Folder", shortcut: { key: "f", meta: true, shift: true }, run: () => this.showSearch() },
      { id: "view.agents", title: "Show Agents", shortcut: { key: "a", meta: true, shift: true }, run: () => this.showAgents() },
      { id: "view.models", title: "Show Models", shortcut: { key: "m", meta: true, shift: true }, run: () => this.showModels() },
      { id: "view.mcp", title: "Show MCP Servers", shortcut: { key: "u", meta: true, shift: true }, run: () => this.showMcp() },
      { id: "view.catalog", title: "Show Catalog", shortcut: { key: "k", meta: true, shift: true }, run: () => this.showCatalog() },
      { id: "terminal.toggle", title: "Toggle Terminal", shortcut: { key: "`", ctrl: true }, run: () => this.toggleTerminal() },
      { id: "terminal.new", title: "New Terminal", shortcut: { key: "`", ctrl: true, shift: true }, run: () => this.newTerminal() },
      { id: "terminal.splitRight", title: "Split Terminal Right", shortcut: { key: "d", meta: true }, when: "terminalFocused", run: () => this.splitTerminal("right") },
      { id: "terminal.splitDown", title: "Split Terminal Down", shortcut: { key: "d", meta: true, shift: true }, when: "terminalFocused", run: () => this.splitTerminal("down") },
      { id: "terminal.closePane", title: "Close Terminal Pane", shortcut: { key: "w", meta: true }, when: "terminalFocused", run: () => void withActive(pane() ?? null, (k) => this.closeTerminalPane(k)) },
      { id: "terminal.nextPane", title: "Focus Next Terminal Pane", shortcut: { key: "]", meta: true }, when: "terminalFocused", run: () => this.terminals.focusNext(1) },
      { id: "terminal.previousPane", title: "Focus Previous Terminal Pane", shortcut: { key: "[", meta: true }, when: "terminalFocused", run: () => this.terminals.focusNext(-1) },
      { id: "explorer.collapse", title: "Collapse Folders in Explorer", run: () => this.explorer.collapseAll() },
    ];
  }

  // Terminal

  toggleTerminal(): void {
    const visible = this.layout.get().terminalVisible;
    this.layout.setTerminalVisible(!visible);
    if (!visible) this.terminals.requestFocus();
  }

  newTerminal(): void {
    this.layout.setTerminalVisible(true);
    this.terminals.add();
  }

  splitTerminal(direction: SplitDirection): void {
    this.layout.setTerminalVisible(true);
    this.terminals.split(direction);
  }

  closeTerminalPane(key: number): void {
    const pane = this.terminals.get().panes.get(key);
    void this.#closeTerminals(() => this.terminals.closePane(key), pane ? [pane] : []);
  }

  closeTerminalTab(key: number): void {
    void this.#closeTerminals(() => this.terminals.closeTab(key), this.#panes(key));
  }

  /** Closes, asking first if that would end a running program or agent. Resolves to whether it closed. */
  async #closeTerminals(close: () => void, panes: readonly TerminalPane[]): Promise<boolean> {
    const { agents, programs } = await this.#busy(panes);
    if (agents.length > 0 && programs === 0) {
      const choice = await this.dialogs.ask({
        title: agents.length === 1 ? `Stop ${agents[0]}?` : `Stop ${agents.length} agents?`,
        message: `Closing ${agents.length === 1 ? "this terminal ends the agent's session" : "these terminals ends their sessions"}.`,
        buttons: [
          { label: "Stop", value: "close", role: "destructive" },
          { label: "Cancel", value: "cancel" },
        ],
        cancel: "cancel",
      });
      if (choice !== "close") return false;
    } else if (agents.length + programs > 0) {
      const busy = agents.length + programs;
      const choice = await this.dialogs.ask({
        title: "End the running program?",
        message:
          busy === 1
            ? "A program is still running in this terminal. Closing it ends the program."
            : `Programs are still running in ${busy} of these terminals. Closing them ends the programs.`,
        buttons: [
          { label: "Close", value: "close", role: "destructive" },
          { label: "Cancel", value: "cancel" },
        ],
        cancel: "cancel",
      });
      if (choice !== "close") return false;
    }
    close();
    return true;
  }

  /** The live panes of one tab, or of every tab. */
  #panes(tabKey?: number): TerminalPane[] {
    const { panes } = this.terminals.get();
    const live = new Set(this.terminals.liveSessions(tabKey));
    return [...panes.values()].filter((pane) => pane.session !== null && live.has(pane.session));
  }

  /**
   * What closing these panes would end: running agents (always worth asking
   * about), and shells running a program besides the shell itself. A session that
   * is gone counts as idle.
   */
  async #busy(panes: readonly TerminalPane[]): Promise<{ agents: string[]; programs: number }> {
    const agents = panes.flatMap((pane) => (pane.kind.type === "agent" && pane.running ? [pane.kind.name] : []));
    const shells = panes.filter((pane) => pane.kind.type === "shell" && pane.session !== null);
    const busy = await Promise.all(shells.map((pane) => this.#native.isTerminalBusy(pane.session!).catch(() => false)));
    return { agents, programs: busy.filter(Boolean).length };
  }

  // Models

  /** ⇧⌘M: model providers in the sidebar. */
  showModels(): void {
    this.layout.showSidebar("models");
    void this.providers.load(true);
  }

  refreshProviders(): void {
    void this.providers.load(true);
  }

  async saveProviderKey(provider: string, key: string): Promise<boolean> {
    const name = this.providers.find(provider)?.name ?? provider;
    try {
      this.providers.replace(await this.#native.setProviderCredential(provider, key));
    } catch (error) {
      this.notifications.error(`Could not save the ${name} key: ${messageOf(error)}`);
      return false;
    }
    this.notifications.info(`${name} key saved in your Keychain.`);
    return true;
  }

  removeProviderKey(provider: string): void {
    void this.#removeProviderKey(provider);
  }

  async #removeProviderKey(provider: string): Promise<void> {
    const name = this.providers.find(provider)?.name ?? provider;
    const choice = await this.dialogs.ask({
      title: `Remove the ${name} key?`,
      message:
        "It is deleted from your Keychain. Agents already running keep working; starting one with this provider needs a key again.",
      buttons: [
        { label: "Remove", value: "remove", role: "destructive" },
        { label: "Cancel", value: "cancel" },
      ],
      cancel: "cancel",
    });
    if (choice !== "remove") return;
    try {
      this.providers.replace(await this.#native.removeProviderCredential(provider));
    } catch (error) {
      this.notifications.error(`Could not remove the ${name} key: ${messageOf(error)}`);
    }
  }

  async addProviderModel(provider: string, model: string): Promise<boolean> {
    try {
      this.providers.replace(await this.#native.addProviderModel(provider, model));
      return true;
    } catch (error) {
      this.notifications.error(`Could not add the model: ${messageOf(error)}`);
      return false;
    }
  }

  removeProviderModel(provider: string, model: string): void {
    this.#native
      .removeProviderModel(provider, model)
      .then((status) => this.providers.replace(status))
      .catch((error: unknown) => this.notifications.error(`Could not remove the model: ${messageOf(error)}`));
  }

  // Context between sessions (docs/multi-agent.md)

  /** The agent session of the focused terminal pane, if it is one. */
  #focusedSession(): number | null {
    const tab = this.terminals.activeTab();
    const pane = tab ? this.terminals.get().panes.get(tab.focused) : undefined;
    return pane?.kind.type === "agent" ? pane.kind.session : null;
  }

  shareContext(mode: ShareMode, session: number | null = null): void {
    void this.#shareContext(mode, session);
  }

  async #shareContext(mode: ShareMode, session: number | null): Promise<void> {
    await this.agents.loadSessions();
    const sessions = this.agents.get().sessions;
    if (sessions.length < 2) {
      this.notifications.info(
        "Context goes from one agent session to another: start a second session first (Agents, or drag a model from the Catalog onto the terminal).",
      );
      return;
    }
    const running = (id: number) => this.terminals.paneOfSession(id)?.running ?? false;
    const chosen = session ?? this.#focusedSession();
    let sources: number[];
    let target: number | null;
    if (mode === "get") {
      target = chosen ?? (sessions.find((s) => running(s.id)) ?? sessions.at(-1)!).id;
      sources = sessions.filter((s) => s.id !== target).map((s) => s.id);
    } else {
      const from = chosen ?? (sessions.find((s) => running(s.id)) ?? sessions[0]!).id;
      const others = sessions.filter((s) => s.id !== from);
      sources = [from];
      target = (others.find((s) => running(s.id)) ?? others[0])?.id ?? null;
    }
    this.#shareChanges.clear();
    this.share.open({ mode, sources, target, parts: DEFAULT_PARTS, note: "", text: "", edited: false, sending: false });
    await this.#composeShare();
  }

  setShareSources(sources: readonly number[]): void {
    const target = this.share.get()?.target ?? null;
    this.share.change({ sources: sources.filter((id) => id !== target) });
    void this.#composeShare();
  }

  setShareTarget(target: number | null): void {
    const state = this.share.get();
    if (!state) return;
    // A session gives or receives, not both.
    this.share.change({ target, sources: state.sources.filter((id) => id !== target) });
    void this.#composeShare();
  }

  setShareParts(parts: ShareParts): void {
    this.share.change({ parts });
    void this.#composeShare();
  }

  setShareNote(note: string): void {
    this.share.change({ note });
    void this.#composeShare();
  }

  editShareText(text: string): void {
    this.share.change({ text, edited: true });
  }

  closeShareContext(): void {
    this.share.close();
  }

  /**
   * The text from the choices: each source's changes (read once per opening of
   * the composer), its terminal's last lines, and the note. Nothing of an
   * agent's own files or transcripts.
   */
  async #composeShare(): Promise<void> {
    const state = this.share.get();
    if (!state) return;
    const { sources, parts, note } = state;
    if (parts.changes || parts.diff) {
      await Promise.all(
        sources
          .filter((id) => !this.#shareChanges.has(id))
          .map(async (id) => {
            try {
              this.#shareChanges.set(id, await this.#native.agentChanges(id));
            } catch (error) {
              this.#shareChanges.set(id, messageOf(error));
            }
          }),
      );
    }
    // Chosen again while the changes were read: that composition wins.
    const now = this.share.get();
    if (!now || now.sources !== sources || now.parts !== parts || now.note !== note) return;
    const text = composeContext(
      sources.flatMap((id) => {
        const session = this.agents.session(id);
        if (!session) return [];
        const changes = this.#shareChanges.get(id);
        const pane = this.terminals.paneOfSession(id);
        const reader = pane ? this.terminals.reader(pane.key) : undefined;
        return [
          {
            name: session.name,
            where: sessionWhere(session),
            changes: typeof changes === "object" ? changes : null,
            changesProblem: typeof changes === "string" ? changes : null,
            includeChanges: parts.changes,
            includeDiff: parts.diff,
            output: parts.output ? (reader?.read(MAX_OUTPUT_LINES * 3) ?? "(its terminal is not open in this window)") : null,
          },
        ];
      }),
      note,
    );
    this.share.change({ text, edited: false });
  }

  sendShareContext(): void {
    void this.#sendShare();
  }

  /**
   * Pastes the text into the receiving agent's input, as a bracketed paste, so
   * nothing runs and nothing is sent until the user presses Enter there. A
   * stopped agent is started first if the user agrees, through the usual
   * approval.
   */
  async #sendShare(): Promise<void> {
    const state = this.share.get();
    if (!state || state.target === null || state.sending) return;
    const target = this.agents.session(state.target);
    const text = pasteable(state.text);
    if (!target || text === "") return;
    const pane = this.terminals.paneOfSession(target.id);
    if (!pane?.running) {
      const choice = await this.dialogs.ask({
        title: `${target.name} is not running`,
        message: "Start it? The context goes into its input once it is ready, and nothing is sent until you press Enter there.",
        buttons: [
          { label: "Start and Send", value: "start", role: "primary" },
          { label: "Cancel", value: "cancel" },
        ],
        cancel: "cancel",
      });
      if (choice !== "start") return;
      this.share.change({ sending: true });
      if (pane) await this.#restartAgent(target.id);
      else await this.#openAgentTerminal(target.id);
      // Not allowed, or not started: the user decided, and there is nothing to wait for.
      if (!this.terminals.paneOfSession(target.id)) {
        this.share.change({ sending: false });
        return;
      }
    } else {
      this.share.change({ sending: true });
    }
    const reader = await this.#readyForPaste(target.id);
    if (!this.share.get()) return;
    if (!reader) {
      this.share.change({ sending: false });
      this.notifications.error(
        `${target.name} did not get ready to take the text, so nothing was sent. Try again once it shows its prompt.`,
      );
      return;
    }
    reader.paste(text);
    this.share.close();
    const shown = this.terminals.paneOfSession(target.id);
    if (shown) {
      this.layout.setTerminalVisible(true);
      this.terminals.focusPane(shown.key);
      this.terminals.requestFocus();
    }
    this.notifications.info(`The context is in ${target.name}'s input: read it there, then press Enter to send it.`);
  }

  /**
   * The session's terminal once its agent takes a bracketed paste and has been
   * quiet for a moment (it shows its prompt), or `null` after a while.
   */
  async #readyForPaste(session: number): Promise<TerminalReader | null> {
    const deadline = Date.now() + PASTE_READY_MS;
    for (;;) {
      const pane = this.terminals.paneOfSession(session);
      // Closed, or ended without starting: nothing will take it.
      if (!pane || (!pane.running && pane.ending)) return null;
      const reader = pane.running ? this.terminals.reader(pane.key) : undefined;
      if (reader?.acceptsPaste() && reader.quietFor() >= QUIET_MS) return reader;
      if (Date.now() >= deadline) return null;
      await new Promise((resolve) => setTimeout(resolve, 200));
    }
  }

  // Catalog

  /** ⇧⌘K: the catalog in the sidebar. It lists; it starts and fetches nothing. */
  showCatalog(): void {
    this.layout.showSidebar("catalog");
    void this.catalog.load();
    void this.skills.load();
  }

  refreshCatalog(): void {
    void this.catalog.load();
    void this.skills.load();
  }

  openAgent(_agent: string): void {
    this.showAgents();
  }

  /**
   * Chooses a catalog item for the next launch of every installed agent that
   * `choose` accepts it for, then shows Agents. Nothing starts: the user presses
   * Launch, and the approval dialog follows as always.
   */
  async #forNextLaunch(what: string, choose: (agent: AgentStatus) => boolean): Promise<void> {
    await Promise.all([this.agents.load(), this.providers.load(), this.mcp.load(), this.skills.load()]);
    const names = (this.agents.get().agents ?? []).filter((a) => a.availability.state === "installed" && choose(a)).map((a) => a.name);
    this.showAgents();
    if (names.length === 0) {
      this.notifications.info(`No installed agent here can use ${what}.`);
    } else {
      this.notifications.info(`${what} is chosen for the next launch of ${names.join(", ")}. Press Launch to start it.`);
    }
  }

  chooseModel(provider: string, model: string): void {
    const name = this.catalog.find(`model.${provider}.${model}`)?.displayName ?? model;
    void this.#forNextLaunch(name, (agent) => {
      const choice = modelChoices(agent, this.providers.get().providers ?? []).find(
        (c) => c.selection.provider === provider && c.selection.model === model,
      );
      if (choice) this.drafts.setModel(agent.id, choice.selection);
      return choice !== undefined;
    });
  }

  launchModel(provider: string, model: string): void {
    void this.#launchModel({ provider, model });
  }

  async #launchModel(selection: ModelSelection): Promise<void> {
    await Promise.all([this.agents.load(), this.providers.load()]);
    const name = this.catalog.find(`model.${selection.provider}.${selection.model}`)?.displayName ?? selection.model;
    const agent = agentForModel(
      this.agents.get().agents ?? [],
      this.providers.get().providers ?? [],
      selection,
      (id) => this.catalog.find(id)?.publisher ?? null,
    );
    if (!agent) {
      this.notifications.info(`No installed agent here can use ${name}. Install one that can (see the Catalog), or save its provider's key in Models.`);
      return;
    }
    this.notifications.info(`Opening ${name} in ${agent.name}.`);
    await this.#launchAgent(agent.id, selection, [], []);
  }

  configureProvider(_provider: string): void {
    this.showModels();
  }

  attachMcp(server: string): void {
    const name = this.catalog.find(`mcp.${server}`)?.displayName ?? this.mcp.find(server)?.server.name ?? server;
    void this.#forNextLaunch(name, (agent) => {
      const offered = mcpChoices(agent, this.mcp.get().servers ?? [], this.workspace.get()?.root ?? null).optional.some(
        (s) => s.server.id === server,
      );
      if (offered) this.drafts.setMcp(agent.id, server, true);
      return offered;
    });
  }

  configureMcp(_server: string): void {
    this.showMcp();
  }

  attachSkill(skill: string): void {
    const name = this.catalog.find(`skill.${skill}`)?.displayName ?? this.skills.find(skill)?.skill.name ?? skill;
    void this.#forNextLaunch(name, (agent) => {
      const offered = skillChoices(agent, this.skills.get().skills ?? [], this.workspace.get()?.root ?? null).optional.some(
        (s) => s.skill.id === skill,
      );
      if (offered) this.drafts.setSkill(agent.id, skill, true);
      return offered;
    });
  }

  async addSkill(skill: SkillInput): Promise<boolean> {
    try {
      await this.#native.addSkill(skill);
    } catch (error) {
      this.notifications.error(`Could not add ${skill.name || "the skill"}: ${messageOf(error)}`);
      return false;
    }
    this.refreshCatalog();
    return true;
  }

  async updateSkill(id: string, skill: SkillInput): Promise<boolean> {
    try {
      await this.#native.updateSkill(id, skill);
    } catch (error) {
      this.notifications.error(`Could not save ${skill.name || "the skill"}: ${messageOf(error)}`);
      return false;
    }
    this.refreshCatalog();
    return true;
  }

  removeSkill(id: string): void {
    void this.#removeSkill(id);
  }

  async #removeSkill(id: string): Promise<void> {
    const name = this.skills.find(id)?.skill.name ?? id;
    const choice = await this.dialogs.ask({
      title: `Remove the skill “${name}”?`,
      message: "Sessions that have it no longer start: they say so, and a new session goes without it.",
      buttons: [
        { label: "Remove", value: "remove", role: "destructive" },
        { label: "Cancel", value: "cancel" },
      ],
      cancel: "cancel",
    });
    if (choice !== "remove") return;
    try {
      await this.#native.removeSkill(id);
    } catch (error) {
      this.notifications.error(`Could not remove ${name}: ${messageOf(error)}`);
      return;
    }
    this.refreshCatalog();
  }

  // MCP

  /** ⇧⌘U: MCP servers in the sidebar. */
  showMcp(): void {
    this.layout.showSidebar("mcp");
    void this.mcp.load();
  }

  refreshMcp(): void {
    void this.mcp.load();
  }

  async addMcpServer(server: McpServerInput): Promise<boolean> {
    try {
      this.mcp.replace(await this.#native.addMcpServer(server));
      return true;
    } catch (error) {
      this.notifications.error(`Could not add ${server.name || "the server"}: ${messageOf(error)}`);
      return false;
    }
  }

  async updateMcpServer(id: string, server: McpServerInput): Promise<boolean> {
    try {
      this.mcp.replace(await this.#native.updateMcpServer(id, server));
      return true;
    } catch (error) {
      this.notifications.error(`Could not save ${server.name || "the server"}: ${messageOf(error)}`);
      return false;
    }
  }

  setMcpServerEnabled(id: string, enabled: boolean): void {
    this.#native
      .setMcpServerEnabled(id, enabled)
      .then((status) => {
        this.mcp.replace(status);
        if (this.catalog.get().items !== null) void this.catalog.load();
      })
      .catch((error: unknown) => this.notifications.error(`Could not change the server: ${messageOf(error)}`));
  }

  removeMcpServer(id: string): void {
    void this.#removeMcpServer(id);
  }

  async #removeMcpServer(id: string): Promise<void> {
    const name = this.mcp.find(id)?.server.name ?? id;
    const choice = await this.dialogs.ask({
      title: `Remove the MCP server “${name}”?`,
      message: "Its secrets are deleted from your Keychain and its approvals forgotten. Sessions that have it no longer start it.",
      buttons: [
        { label: "Remove", value: "remove", role: "destructive" },
        { label: "Cancel", value: "cancel" },
      ],
      cancel: "cancel",
    });
    if (choice !== "remove") return;
    try {
      await this.#native.removeMcpServer(id);
      this.mcp.forget(id);
    } catch (error) {
      this.notifications.error(`Could not remove ${name}: ${messageOf(error)}`);
    }
  }

  async saveMcpSecret(id: string, name: string, value: string): Promise<boolean> {
    try {
      this.mcp.replace(await this.#native.setMcpSecret(id, name, value));
    } catch (error) {
      this.notifications.error(`Could not save ${name}: ${messageOf(error)}`);
      return false;
    }
    this.notifications.info(`${name} saved in your Keychain.`);
    return true;
  }

  removeMcpSecret(id: string, name: string): void {
    void this.#removeMcpSecret(id, name);
  }

  async #removeMcpSecret(id: string, name: string): Promise<void> {
    const choice = await this.dialogs.ask({
      title: `Remove the saved ${name}?`,
      message: "It is deleted from your Keychain. The server does not start again until it is saved.",
      buttons: [
        { label: "Remove", value: "remove", role: "destructive" },
        { label: "Cancel", value: "cancel" },
      ],
      cancel: "cancel",
    });
    if (choice !== "remove") return;
    try {
      this.mcp.replace(await this.#native.removeMcpSecret(id, name));
    } catch (error) {
      this.notifications.error(`Could not remove ${name}: ${messageOf(error)}`);
    }
  }

  // Agents

  /** ⇧⌘A: the agents in the sidebar. */
  showAgents(): void {
    this.layout.showSidebar("agents");
    void this.agents.load();
    void this.providers.load();
    void this.mcp.load();
    void this.skills.load();
  }

  refreshAgents(): void {
    void this.agents.load(true);
  }

  trustWorkspace(): void {
    void this.setTrust(true);
  }

  launchAgent(id: string, model: ModelSelection | null = null, mcp: readonly string[] = [], skills: readonly string[] = []): void {
    void this.#launchAgent(id, model, mcp, skills);
  }

  /**
   * An agent starts only in a trusted folder, and only once the user allowed it
   * there. Both are enforced natively; this walks the user through them: trust
   * first (asked here, granted in a native dialog), then approval (a native
   * dialog, which names the provider and endpoint when a model is chosen). Then
   * the native side makes the agent a session (a worktree of its own in a Git
   * repository) that keeps the model, and a terminal pane starts the agent in it.
   */
  async #launchAgent(id: string, model: ModelSelection | null, mcp: readonly string[], skills: readonly string[]): Promise<void> {
    const workspace = this.workspace.get();
    if (!workspace) {
      this.notifications.info("Open a folder first (⌘O). Agents run in the open folder.");
      return;
    }
    const name = this.agents.find(id)?.name ?? id;
    if (!workspace.trusted) {
      const choice = await this.dialogs.ask({
        title: `“${workspace.name}” is not trusted`,
        message: `${name} can run only in folders you trust. Trust a folder only if you trust the code in it.`,
        buttons: [
          { label: "Trust Folder…", value: "trust", role: "primary" },
          { label: "Cancel", value: "cancel" },
        ],
        cancel: "cancel",
      });
      if (choice !== "trust") return;
      await this.setTrust(true);
      if (!this.workspace.get()?.trusted) return;
    }
    let approved: boolean;
    try {
      approved = await this.#native.requestAgentApproval(id, model, mcp, skills);
    } catch (error) {
      this.notifications.error(`Could not start ${name}: ${messageOf(error)}`);
      return;
    }
    this.#reloadAgents();
    if (!approved) return;
    let session: AgentSessionInfo;
    try {
      session = await this.#native.createAgentSession(id, model, mcp, skills);
    } catch (error) {
      this.notifications.error(`Could not start ${name}: ${messageOf(error)}`);
      return;
    }
    void this.agents.loadSessions();
    this.layout.setTerminalVisible(true);
    this.terminals.add({ type: "agent", agent: id, name, session: session.id });
  }

  /** Shows the session's terminal; a new one starts the agent again if it is not running. */
  openAgentTerminal(id: number): void {
    void this.#openAgentTerminal(id);
  }

  async #openAgentTerminal(id: number): Promise<void> {
    const session = this.agents.session(id);
    const pane = this.terminals.paneOfSession(id);
    this.layout.setTerminalVisible(true);
    if (pane) {
      this.terminals.focusPane(pane.key);
      this.terminals.requestFocus();
      return;
    }
    if (!session) return;
    // Running it again runs what it has now: asked for if not yet allowed (a
    // changed MCP server, a revoked approval).
    try {
      if (!(await this.#native.requestSessionApproval(id))) return;
    } catch (error) {
      this.notifications.error(`Could not start ${session.name}: ${messageOf(error)}`);
      return;
    }
    this.terminals.add({ type: "agent", agent: session.agent, name: session.name, session: id });
  }

  stopAgent(id: number): void {
    void this.#stopAgent(id);
  }

  /** Resolves to whether the agent was stopped (or was not running). */
  async #stopAgent(id: number, ask = true): Promise<boolean> {
    const session = this.agents.session(id);
    if (!session || session.state.state !== "running") return true;
    if (ask) {
      const choice = await this.dialogs.ask({
        title: `Stop ${session.name}?`,
        message: "Its terminal session ends. Its workspace and changes stay.",
        buttons: [
          { label: "Stop", value: "stop", role: "destructive" },
          { label: "Cancel", value: "cancel" },
        ],
        cancel: "cancel",
      });
      if (choice !== "stop") return false;
    }
    try {
      await this.#native.stopAgentSession(id);
    } catch (error) {
      this.notifications.error(`Could not stop ${session.name}: ${messageOf(error)}`);
      return false;
    }
    await this.#paneEnded(id);
    await this.agents.loadSessions();
    return true;
  }

  restartAgent(id: number): void {
    void this.#restartAgent(id);
  }

  async #restartAgent(id: number): Promise<void> {
    if (!(await this.#stopAgent(id))) return;
    const pane = this.terminals.paneOfSession(id);
    if (pane) this.terminals.closePane(pane.key);
    await this.#openAgentTerminal(id);
  }

  /** Waits (briefly) until the session's pane reports its agent has ended. */
  async #paneEnded(id: number): Promise<void> {
    const ended = () => !this.terminals.paneOfSession(id)?.running;
    for (let i = 0; i < 50 && !ended(); i++) await new Promise((resolve) => setTimeout(resolve, 100));
  }

  showAgentChanges(id: number): void {
    void this.#showAgentChanges(id);
  }

  async #showAgentChanges(id: number): Promise<void> {
    const session = this.agents.session(id);
    if (!session) return;
    let changes: AgentChanges;
    try {
      changes = await this.#native.agentChanges(id);
    } catch (error) {
      this.notifications.error(`Could not read ${session.name}'s changes: ${messageOf(error)}`);
      return;
    }
    this.agents.setChanges(id, changes);
    const where = changes.branch ?? session.worktree?.branch ?? session.name;
    const summary = [
      `# ${session.name} · ${where}`,
      `# ${changes.files.length} changed file${changes.files.length === 1 ? "" : "s"}, ${changes.commits} commit${changes.commits === 1 ? "" : "s"} since ${changes.base.slice(0, 10)}`,
      ...(changes.truncated ? ["# The diff is too large to show in full."] : []),
      "",
    ].join("\n");
    this.editor.openReadOnly(
      `/agent/${id}/changes.diff`,
      `${session.name} changes`,
      `${session.name}: changes on ${where} (read-only)`,
      changes.diff === "" ? `${summary}\n# No changes yet.\n` : `${summary}\n${changes.diff}`,
    );
  }

  openAgentFile(id: number, path: string): void {
    void this.#openAgentFile(id, path);
  }

  async #openAgentFile(id: number, path: string): Promise<void> {
    const session = this.agents.session(id);
    if (!session) return;
    try {
      const content = await this.#native.readAgentFile(id, path);
      const where = session.worktree?.branch ?? session.name;
      this.editor.openReadOnly(`/agent/${id}/${path}`, `${basename(path)} · ${session.name}`, `${where}: ${path} (read-only)`, content.text);
    } catch (error) {
      this.notifications.error(`Could not open ${path}: ${messageOf(error)}`);
    }
  }

  removeAgentSession(id: number): void {
    void this.#removeAgentSession(id);
  }

  /**
   * Removes a stopped session and its worktree. Asks first, and says exactly what
   * goes: uncommitted changes are discarded only if the user confirms; a branch
   * with commits is always kept.
   */
  async #removeAgentSession(id: number): Promise<void> {
    const session = this.agents.session(id);
    if (!session) return;
    if (session.state.state === "running") {
      this.notifications.info(`Stop ${session.name} before removing its workspace.`);
      return;
    }
    let changes: AgentChanges | null = null;
    if (session.worktree) {
      try {
        changes = await this.#native.agentChanges(id);
      } catch (error) {
        this.notifications.error(`Could not read ${session.name}'s changes: ${messageOf(error)}`);
        return;
      }
    }
    const uncommitted = changes?.uncommitted ?? false;
    const lines = [
      session.worktree ? `The worktree at ${session.worktree.path} is deleted.` : "The session is forgotten; your folder is not changed.",
      ...(changes && changes.commits > 0 ? [`The branch ${session.worktree?.branch} and its ${changes.commits} commit${changes.commits === 1 ? "" : "s"} are kept.`] : []),
      ...(changes && uncommitted ? ["Changes that were not committed are lost."] : []),
    ];
    const choice = await this.dialogs.ask({
      title: session.worktree ? `Remove ${session.name}'s workspace?` : `Remove ${session.name}'s session?`,
      message: lines.join(" "),
      buttons: [
        { label: uncommitted ? "Discard and Remove" : "Remove", value: "remove", role: "destructive" },
        { label: "Cancel", value: "cancel" },
      ],
      cancel: "cancel",
    });
    if (choice !== "remove") return;
    try {
      const removal = await this.#native.removeAgentSession(id, uncommitted);
      const pane = this.terminals.paneOfSession(id);
      if (pane) this.terminals.closePane(pane.key);
      this.editor.get().tabs.filter((t) => t.path.startsWith(`/agent/${id}/`)).forEach((t) => this.editor.close(t.path));
      this.notifications.info(
        removal.keptBranch ? `Removed. The branch ${removal.keptBranch} is kept.` : `Removed ${session.name}'s workspace.`,
      );
    } catch (error) {
      this.notifications.error(`Could not remove ${session.name}'s workspace: ${messageOf(error)}`);
    }
    await this.agents.loadSessions();
  }

  /** Agent panes that start or end change what the sessions report. */
  #agentPanesChanged(): void {
    const running = this.terminals
      .agentPanes()
      .map((p) => `${p.key}:${p.running}`)
      .join(",");
    if (running === this.#agentPanesSeen) return;
    this.#agentPanesSeen = running;
    if (this.agents.get().agents !== null) void this.agents.loadSessions();
  }

  revokeAgent(id: string): void {
    this.#native
      .revokeAgentApproval(id)
      .catch((error: unknown) => this.notifications.error(`Could not revoke the approval: ${messageOf(error)}`))
      .finally(() => this.#reloadAgents());
  }

  /** Resolves to whether it is fine to stop the running agents, which belong to the open folder. */
  async #confirmStopAgents(
    why = "Agents run only in the folder they were allowed in. Opening another folder stops them.",
  ): Promise<boolean> {
    const running = this.terminals.agentPanes().filter((pane) => pane.running);
    if (running.length === 0) return true;
    const names = [...new Set(running.map((pane) => (pane.kind.type === "agent" ? pane.kind.name : "")))];
    const choice = await this.dialogs.ask({
      title: names.length === 1 ? `Stop ${names[0]}?` : `Stop ${running.length} agents?`,
      message: why,
      buttons: [
        { label: "Stop and Continue", value: "stop", role: "destructive" },
        { label: "Cancel", value: "cancel" },
      ],
      cancel: "cancel",
    });
    return choice === "stop";
  }

  /** Refreshes the agent list, if it has been shown: its approvals depend on the folder and its trust. */
  #reloadAgents(): void {
    if (this.agents.get().agents !== null) void this.agents.load();
  }

  // Quitting

  /**
   * The native side asks before quitting when there are unsaved changes or a
   * terminal is running a program. Both are confirmed here, then the app quits.
   */
  async #quitRequested(): Promise<void> {
    const dirty = this.editor.dirtyPaths();
    if (dirty.length > 0) {
      const choice = await this.dialogs.ask({
        title:
          dirty.length === 1
            ? `Save changes to "${basename(dirty[0]!)}" before quitting?`
            : `Save changes to ${dirty.length} files before quitting?`,
        message: "Your changes will be lost if you don't save them.",
        buttons: [
          { label: dirty.length === 1 ? "Save" : "Save All", value: "save", role: "primary" },
          { label: "Don't Save", value: "discard", role: "destructive" },
          { label: "Cancel", value: "cancel" },
        ],
        cancel: "cancel",
      });
      if (choice === "cancel") return;
      if (choice === "save" && !(await this.saveAll())) return;
    }
    const { agents, programs } = await this.#busy(this.#panes());
    const busy = agents.length + programs;
    if (busy > 0) {
      const what = agents.length === 1 && programs === 0 ? agents[0]! : busy === 1 ? "the running program" : `${busy} running programs`;
      const choice = await this.dialogs.ask({
        title: agents.length === 1 && programs === 0 ? `Quit and stop ${what}?` : `Quit and end ${what}?`,
        message:
          busy === 1
            ? `${agents.length === 1 ? what : "A program"} is still running in a terminal. Quitting ends it.`
            : `Programs are still running in ${busy} terminals. Quitting ends them.`,
        buttons: [
          { label: "Quit", value: "quit", role: "destructive" },
          { label: "Cancel", value: "cancel" },
        ],
        cancel: "cancel",
      });
      if (choice !== "quit") return;
    }
    await this.#native.quit();
  }

  /** Resolves to whether it is fine to discard all open tabs. */
  async #confirmDiscard(context: string): Promise<boolean> {
    const dirty = this.editor.dirtyPaths();
    if (dirty.length === 0) return true;
    const choice = await this.dialogs.ask({
      title: `Save changes to ${dirty.length === 1 ? `"${basename(dirty[0]!)}"` : `${dirty.length} files`}?`,
      message: `${context} Your changes will be lost if you don't save them.`,
      buttons: [
        { label: dirty.length === 1 ? "Save" : "Save All", value: "save", role: "primary" },
        { label: "Don't Save", value: "discard", role: "destructive" },
        { label: "Cancel", value: "cancel" },
      ],
      cancel: "cancel",
    });
    if (choice === "cancel") return false;
    return choice === "discard" || (await this.saveAll());
  }

  #reportUnsaved(): void {
    const unsaved = this.editor.dirtyPaths().length > 0;
    if (unsaved === this.#reportedUnsaved) return;
    this.#reportedUnsaved = unsaved;
    this.#native.setUnsavedChanges(unsaved).catch((error: unknown) => {
      this.notifications.error(`Could not protect unsaved changes on quit: ${messageOf(error)}`);
    });
  }
}

function withActive<T>(value: T | null, run: (value: T) => void): void {
  if (value !== null) run(value);
}
