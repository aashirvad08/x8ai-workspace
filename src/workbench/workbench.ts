import type { DirEntry } from "../contracts/generated/DirEntry";
import type { RecentWorkspace } from "../contracts/generated/RecentWorkspace";
import type { SearchMatch } from "../contracts/generated/SearchMatch";
import type { SessionId } from "../contracts/generated/SessionId";
import type { WorkspaceEvent } from "../contracts/generated/WorkspaceEvent";
import type { WorkspaceInfo } from "../contracts/generated/WorkspaceInfo";
import type { EditorActions } from "../editor/actions";
import { EditorStore } from "../editor/editor-store";
import { basename, dirname, join } from "../lib/paths";
import { Value } from "../lib/store";
import { type NativeClient, NativeError } from "../native";
import type { TerminalActions } from "../terminal/actions";
import type { SplitDirection } from "../terminal/panes";
import { Terminals } from "../terminal/terminals";
import type { ExplorerActions, SearchActions } from "../workspace/actions";
import { Explorer } from "../workspace/explorer";
import { Search } from "../workspace/search";
import type { Command } from "./commands";
import { Dialogs } from "./dialogs";
import { Layout } from "./layout";
import { messageOf, Notifications } from "./notifications";
import { Picker } from "./picker";

/**
 * The workspace as the user works with it: the open folder, its file tree, editor
 * tabs, terminals, search, and everything that coordinates them. Every user
 * action lives here, so components stay presentational and the behaviour is
 * testable without React.
 */
export class Workbench implements ExplorerActions, EditorActions, TerminalActions, SearchActions {
  readonly workspace = new Value<WorkspaceInfo | null>(null);
  /** Recently opened folders, most recent first. */
  readonly recent = new Value<readonly RecentWorkspace[]>([]);
  readonly explorer: Explorer;
  readonly editor: EditorStore;
  readonly search: Search;
  readonly terminals = new Terminals();
  readonly notifications = new Notifications();
  readonly dialogs = new Dialogs();
  readonly layout = new Layout();
  readonly picker = new Picker();

  readonly #native: NativeClient;
  /** Identifies the shown workspace's disk events (see `#open`). */
  #shown: symbol | null = null;
  #files: { paths: readonly string[]; truncated: boolean } | null = null;
  #reportedUnsaved = false;

  constructor(native: NativeClient) {
    this.#native = native;
    this.explorer = new Explorer(native);
    this.editor = new EditorStore(native);
    this.search = new Search(native);
    this.editor.subscribe(() => this.#reportUnsaved());
  }

  /**
   * Connects to the native host: quit requests and startup warnings. Reopens the
   * most recent workspace if it still exists, then starts a first terminal (in
   * that workspace, if one opened).
   */
  async start(): Promise<void> {
    this.#native.subscribeApp((event) => {
      if (event.type === "quitRequested") void this.#quitRequested();
    }).catch((error: unknown) => this.notifications.error(`Could not connect to the app: ${messageOf(error)}`));
    await this.#refreshRecent();
    const last = this.recent.get()[0];
    if (last?.available) await this.#open((listener) => this.#native.openRecentWorkspace(last.root, listener));
    if (this.terminals.get().tabs.length === 0) this.terminals.add();
    await this.#showWarnings();
  }

  // Workspace

  async openFolder(): Promise<void> {
    if (!(await this.#confirmDiscard("Opening another folder closes its open files."))) return;
    await this.#open((listener) => this.#native.openWorkspace(listener));
  }

  openRecent(root: string): void {
    void this.#openRecent(root);
  }

  async #openRecent(root: string): Promise<void> {
    if (this.workspace.get()?.root === root) return;
    if (!(await this.#confirmDiscard("Opening another folder closes its open files."))) return;
    await this.#open((listener) => this.#native.openRecentWorkspace(root, listener));
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
        message: "The folder goes back to untrusted. Nothing in it is changed.",
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
      if (this.workspace.get()?.root === info.root) this.workspace.set(info);
    } catch (error) {
      this.notifications.error(`Could not change trust: ${messageOf(error)}`);
    }
  }

  /**
   * Opens a workspace and shows it. `null` (a cancelled picker) keeps the current
   * one. Disk events are followed only for the workspace that is shown: events
   * still arriving from a previous one are ignored.
   */
  async #open(opening: (listener: (event: WorkspaceEvent) => void) => Promise<WorkspaceInfo | null>): Promise<void> {
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
      return;
    } finally {
      await this.#showWarnings();
    }
    if (!info) return;
    this.#shown = token;
    this.editor.closeAll();
    this.#files = null;
    this.workspace.set(info);
    this.explorer.reset(true);
    this.search.reset();
    // A new session starts in the new workspace. Existing ones stay where they are.
    this.terminals.add();
    await this.#refreshRecent();
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
    void this.#closeTerminals(() => this.terminals.closePane(key), this.#sessionsOfPane(key));
  }

  closeTerminalTab(key: number): void {
    void this.#closeTerminals(() => this.terminals.closeTab(key), this.terminals.liveSessions(key));
  }

  /** Closes, asking first if that would end a running program. Resolves to whether it closed. */
  async #closeTerminals(close: () => void, sessions: readonly SessionId[]): Promise<boolean> {
    const busy = await this.#countBusy(sessions);
    if (busy > 0) {
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

  #sessionsOfPane(key: number): SessionId[] {
    const session = this.terminals.get().panes.get(key)?.session;
    return session === undefined || session === null ? [] : [session];
  }

  /** How many sessions run a program besides their shell. A session that is gone counts as idle. */
  async #countBusy(sessions: readonly SessionId[]): Promise<number> {
    const busy = await Promise.all(sessions.map((id) => this.#native.isTerminalBusy(id).catch(() => false)));
    return busy.filter(Boolean).length;
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
    const busy = await this.#countBusy(this.terminals.liveSessions());
    if (busy > 0) {
      const choice = await this.dialogs.ask({
        title: busy === 1 ? "Quit and end the running program?" : `Quit and end ${busy} running programs?`,
        message:
          busy === 1
            ? "A program is still running in a terminal. Quitting ends it."
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
