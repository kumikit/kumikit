/**
 * The UI kernel's state: the connections (one per core), split layout and tabs, threads,
 * commands, slots, theme and settings, and the kernel's own commands. Components read it;
 * nothing here names a plugin.
 */
import type { Attachment, PermissionMode, Thread } from '@kumikit/protocol';
import type { Connection, Slot, ThreadAddress } from '@kumikit/ui-sdk';

import { CommandRegistry, type WhenContext } from './commands/registry.svelte';
import { CoreCommands, type CoreView } from './commands/core.svelte';
import { ClientPlugins } from './plugins/client.svelte';
import { CorePlugins } from './plugins/core.svelte';
import { Registries } from './plugins/registry.svelte';
import { CoreConnection, type Timers } from './connections/connection.svelte';
import {
  LEFT_MAX,
  LEFT_MIN,
  RIGHT_MAX,
  RIGHT_MIN,
  allLeaves,
  clamp,
  clampTabIndex,
  closeLeaf,
  countLeaves,
  findLeaf,
  firstLeaf,
  layoutCss,
  setSplitRatio,
  splitLeaf,
  updateLeaf,
  type LayoutLeaf,
  type LayoutNode,
  type LeftSidebarView,
  type SplitDirection,
} from './layout/layout';
import type { PickedAttachment, Platform, ProfileInfo, WindowChrome } from './platform/index';
import { meetsRequirements } from './plugins/requires';
import type { Command } from './commands/registry.svelte';
import type { Entry } from './slots/registry.svelte';
import { RpcError } from './rpc/client';
import { SettingsStore, type TabSpec } from './settings/settings.svelte';
import { SlotRegistry } from './slots/registry.svelte';
import { ThemeEngine, variantKey, type ThemeVariant } from './theme/engine.svelte';
import { parseThemeFile, parseThemeObject } from './theme/file';
import { ThreadStore } from './threads/store.svelte';
import { UpdateManager } from './updates/updates.svelte';
import { pushRecent } from './ui/model-picker';

export type Tone = 'info' | 'success' | 'warning' | 'danger';

export interface Toast {
  id: number;
  message: string;
  tone: Tone;
}

/** What the main area shows. */
export type MainView =
  | { kind: 'tab' }
  | { kind: 'settings'; page?: string }
  | { kind: 'plugins'; starter?: boolean }
  | { kind: 'cores' }
  | { kind: 'view'; id: string; args?: unknown }
  | { kind: 'core-view'; view: CoreView };

/** A dialog the kernel shows over everything. */
export type Dialog =
  | { kind: 'palette' }
  | { kind: 'open-folder'; connection: string }
  | { kind: 'rename-thread'; address: ThreadAddress; title: string }
  | {
      kind: 'confirm';
      title: string;
      message: string;
      confirmLabel: string;
      tone?: 'danger' | 'primary';
      onConfirm: () => Promise<void> | void;
    }
  | undefined;

/** The home profile before the shell lists profiles. */
function homeProfile(platform: Platform): Omit<ProfileInfo, 'name'> {
  return platform.home === 'local' ? { label: 'This computer', kind: 'local' } : { label: 'This core', kind: 'core' };
}

export interface KernelOptions {
  platform: Platform;
  storage?: Storage;
  document?: Document;
  systemDark?: () => boolean;
  mac?: boolean;
  /** Every connection's timers (tests control backoff with them). */
  timers?: Timers;
}

export class Kernel {
  readonly platform: Platform;
  /** One per core, the platform's home profile first. */
  connections = $state.raw<CoreConnection[]>([]);
  readonly threads: ThreadStore;
  readonly commands = new CommandRegistry();
  readonly slots: SlotRegistry;
  readonly theme: ThemeEngine;
  readonly settings: SettingsStore;
  readonly mac: boolean;
  readonly clientPlugins: ClientPlugins;
  readonly corePlugins: CorePlugins;
  readonly registries: Registries;
  readonly updates: UpdateManager;
  main = $state.raw<MainView>({ kind: 'tab' });
  /** Core plugins' commands for the active tab's core. */
  readonly coreCommands: CoreCommands;
  dialog = $state.raw<Dialog>(undefined);
  /** Desktop window chrome configuration (undefined in browser/relay). */
  windowChrome = $state.raw<WindowChrome | undefined>(undefined);
  windowMaximized = $state(false);
  windowFullscreen = $state(false);
  /** Bumped whenever the user explicitly starts a draft thread so the inline composer focuses. */
  draftFocusNonce = $state(0);
  toasts = $state.raw<Toast[]>([]);
  #toastId = 0;
  #systemDark: () => boolean;
  #unwatch = new Map<string, () => void>();
  #unlistenWindow: (() => void) | undefined;
  #timers: Timers | undefined;
  #doc: Document | undefined;
  #layoutSheet: CSSStyleSheet | undefined;

  constructor(options: KernelOptions) {
    this.platform = options.platform;
    this.mac = options.mac ?? false;
    this.#timers = options.timers;
    this.#doc = options.document;
    this.connections = [new CoreConnection(options.platform.home, options.platform, this.#options(homeProfile(options.platform)))];
    this.threads = new ThreadStore((id) => this.connectionFor(id));
    this.slots = new SlotRegistry((plugin, reason) => this.onPluginFailed(plugin, reason));
    this.theme = new ThemeEngine(options.document);
    this.settings = new SettingsStore(options.storage);
    this.#systemDark = options.systemDark ?? (() => false);
    this.clientPlugins = new ClientPlugins(this);
    this.corePlugins = new CorePlugins(this);
    this.registries = new Registries(this);
    this.updates = new UpdateManager(this.platform, this.settings, () => this.connections[0], {
      ...(this.#timers ? { timers: this.#timers } : {}),
      notify: (m, t) => this.notify(m, t),
    });
    this.coreCommands = new CoreCommands(this);
    this.#registerCommands();
    this.coreCommands.watch();
  }

  /** Called when the slot registry disables a plugin; the plugin manager replaces it. */
  onPluginFailed = (plugin: string, reason: string): void => {
    this.notify(`The plugin ${plugin} was disabled after repeated failures: ${reason}`, 'danger');
    void this.clientPlugins.failed(plugin);
  };

  async start(): Promise<void> {
    this.applyTheme();
    this.applyLayoutStyles();
    // Plugins and every core start side by side; none waits for another.
    const plugins = this.clientPlugins.load();
    const profiles = this.platform.profiles.list().catch((e: unknown) => {
      this.notify(`Cannot list the cores: ${e instanceof Error ? e.message : String(e)}`, 'danger');
      return [] as ProfileInfo[];
    });
    const home = this.connections[0]!.start();
    for (const p of await profiles) if (!this.connectionFor(p.name)) this.#add(p);
    for (const leaf of allLeaves(this.layoutRoot)) {
      for (const tab of leaf.tabs) this.#watch(tab.connection, tab.workspace);
    }
    const updates = this.updates.start();
    const chrome = this.#initWindowChrome();
    await Promise.all([home, plugins, updates, chrome]);
  }

  /** Whether custom minimize/maximize/close buttons should show in the top bar. */
  get showWindowButtons(): boolean {
    if (!this.platform.window || !this.windowChrome) return false;
    if (this.windowChrome.nativeOverlay || !this.windowChrome.borderless) return false;
    const mode = this.settings.value.windowButtons;
    if (mode === 'show') return true;
    if (mode === 'hide') return false;
    return !this.windowChrome.tiling;
  }

  /** Whether to reserve left padding in the top bar for macOS native overlay traffic lights. */
  get nativeOverlayInset(): boolean {
    return Boolean(this.windowChrome?.nativeOverlay && !this.windowFullscreen);
  }

  /** Whether thin invisible edge strips should handle resizing (undecorated Linux stacking DE). */
  get showCustomResizeHandles(): boolean {
    return Boolean(this.platform.window && this.windowChrome?.customResize && !this.windowMaximized && !this.windowFullscreen);
  }

  /** Whether dragging empty space in the top bar moves the window. */
  get canDragTopBar(): boolean {
    return Boolean(this.platform.window && (this.windowChrome?.borderless || this.windowChrome?.nativeOverlay));
  }

  async #initWindowChrome(): Promise<void> {
    const win = this.platform.window;
    if (!win) return;
    try {
      this.windowChrome = await win.chrome();
    } catch {
      return;
    }
    const syncState = async () => {
      try {
        const [max, full] = await Promise.all([win.isMaximized(), win.isFullscreen()]);
        this.windowMaximized = max;
        this.windowFullscreen = full;
      } catch {
        // Window state read is best-effort.
      }
    };
    await syncState();
    try {
      this.#unlistenWindow?.();
      this.#unlistenWindow = await win.onStateChange(() => {
        void syncState();
      });
    } catch {
      // Resize listener is best-effort.
    }
  }

  /** Updates the constructed stylesheet carrying sidebar widths and split ratios (CSP-safe). */
  applyLayoutStyles(): void {
    const doc = this.#doc;
    if (!doc || !('adoptedStyleSheets' in doc) || typeof CSSStyleSheet !== 'function') return;
    const s = this.settings.value;
    const css = layoutCss(s.leftSidebarWidth, s.rightSidebarWidth, s.layout);
    if (!this.#layoutSheet) {
      this.#layoutSheet = new CSSStyleSheet();
      doc.adoptedStyleSheets = [...doc.adoptedStyleSheets, this.#layoutSheet];
    }
    this.#layoutSheet.replaceSync(css);
  }

  // ---- connections ----

  /** The connection with this id, if the core is known. */
  connectionFor(id: string | undefined): CoreConnection | undefined {
    return id === undefined ? undefined : this.connections.find((c) => c.id === id);
  }

  /** The active tab's connection, or the home one. Commands without a tab act here. */
  get connection(): CoreConnection {
    return this.connectionFor(this.activeTab?.connection) ?? this.connections[0]!;
  }

  #add(profile: ProfileInfo): CoreConnection {
    const prof: Omit<ProfileInfo, 'name'> = {
      label: profile.label,
      kind: profile.kind,
      ...(profile.autoSync !== undefined ? { autoSync: profile.autoSync } : {}),
    };
    const c = new CoreConnection(profile.name, this.platform, this.#options(prof));
    this.connections = [...this.connections, c];
    void c.start().then(() => this.#maybeAutoSyncSsh(c));
    return c;
  }

  /** Tracks which SSH connections already triggered an automatic sync attempt in this session. */
  #sshAutoSynced = new Set<string>();

  async #maybeAutoSyncSsh(c: CoreConnection): Promise<void> {
    if (c.kind !== 'ssh' || c.status !== 'mismatch') return;
    if (!this.settings.value.autoSyncSsh && !c.autoSync) return;
    if (!this.platform.profiles.syncSsh || this.#sshAutoSynced.has(c.id)) return;
    this.#sshAutoSynced.add(c.id);
    this.notify(`Installing matching Kumikit on ${c.label} over SSH…`, 'info');
    try {
      const res = await this.platform.profiles.syncSsh({ profile: c.id, ifIdle: true });
      this.notify(`Installed Kumikit ${res.installedVersion} on ${c.label}. Reconnecting…`, 'success');
      c.reconnectNow();
    } catch (e) {
      this.notify(
        `Cannot auto-sync ${c.label}: ${e instanceof Error ? e.message : String(e)}`,
        'warning',
      );
    }
  }

  /** Updates an SSH profile's per-host auto-sync setting. */
  async setCoreAutoSync(name: string, autoSync: boolean): Promise<boolean> {
    if (!this.platform.profiles.updateSsh) return false;
    const c = this.connectionFor(name);
    if (!c || c.kind !== 'ssh') return false;
    const updated = await this.attempt('Cannot update SSH core setting', () =>
      this.platform.profiles.updateSsh!({ name, autoSync }),
    );
    if (!updated) return false;
    c.autoSync = updated.autoSync === true;
    return true;
  }

  #options(profile: Omit<ProfileInfo, 'name'>): ConstructorParameters<typeof CoreConnection>[2] {
    return this.#timers ? { profile, timers: this.#timers } : { profile };
  }

  /** Records a core the shell just added, and connects to it. */
  addedCore(profile: ProfileInfo): CoreConnection {
    return this.connectionFor(profile.name) ?? this.#add(profile);
  }

  /** Forgets a core: the shell revokes this device there if it can. Its tabs stay, saying
   * the core was removed, until the user closes them. */
  async removeCore(name: string): Promise<boolean> {
    const c = this.connectionFor(name);
    if (!c || name === this.platform.home) return false;
    const r = await this.attempt('Cannot remove the core', () => this.platform.profiles.remove(name));
    if (!r) return false;
    c.stop();
    this.connections = this.connections.filter((x) => x !== c);
    for (const [key, stop] of this.#unwatch) {
      if (key.startsWith(`${name}/`)) {
        stop();
        this.#unwatch.delete(key);
      }
    }
    if (r.revoked) this.notify(`Removed ${c.label}`, 'success');
    else
      this.notify(
        `Removed ${c.label}, but the core could not be reached. On that machine, run \`kumikit devices revoke ${r.deviceId ?? ''}\`.`,
        'warning',
      );
    return true;
  }

  // ---- what plugins may contribute where (07-ui-plugins, requires.core) ----

  /** Whether a plugin's `requires.core` is met on a connection. */
  pluginAllowed(plugin: string, connection: string | undefined): boolean {
    const requires = this.clientPlugins.requiresOf(plugin);
    if (!requires || requires.length === 0) return true;
    const c = this.connectionFor(connection);
    return c !== undefined && meetsRequirements(requires, c.plugins);
  }

  /** The contributions to a slot that apply on a connection (the active tab's by default). */
  slotsIn<S extends Slot>(slot: S, connection: string | undefined = this.activeTab?.connection ?? this.connection.id): Entry<S>[] {
    return this.slots.in(slot).filter((e) => this.pluginAllowed(e.plugin, connection));
  }

  /** The renderer for an item kind on a connection. */
  rendererFor(kind: string, connection: string | undefined): Entry<'transcript.renderer'> | undefined {
    return this.slots.renderer(kind, (plugin) => this.pluginAllowed(plugin, connection));
  }

  /** Whether a command is offered now: kernel commands always, plugin ones only where
   * their `requires.core` is met. */
  commandAllowed = (c: Command): boolean => c.source === 'kernel' || this.pluginAllowed(c.source, this.connection.id);

  /** Links never navigate the app (06-client, UI rules): a link the markdown renderer
   * marked is copied, and any other link click is ignored. */
  guardLinks(doc: Document, clipboard: { writeText(t: string): Promise<void> } | undefined): () => void {
    const onClick = (e: MouseEvent) => {
      const a = (e.target as Element | null)?.closest?.('a[href]');
      if (!a) return;
      e.preventDefault();
      if (!a.hasAttribute('data-kk-link')) return;
      const href = a.getAttribute('href') ?? '';
      void (clipboard?.writeText(href) ?? Promise.reject(new Error('no clipboard'))).then(
        () => this.notify(`Link copied: ${href}`, 'success'),
        () => this.notify(`Link: ${href}`, 'info'),
      );
    };
    doc.addEventListener('click', onClick, true);
    doc.addEventListener('auxclick', onClick, true);
    return () => {
      doc.removeEventListener('click', onClick, true);
      doc.removeEventListener('auxclick', onClick, true);
    };
  }

  // ---- layout, sidebars & split panes ----

  get layoutRoot(): LayoutNode {
    return this.settings.value.layout;
  }

  get activePaneId(): string {
    return this.activePane.id;
  }

  get activePane(): LayoutLeaf {
    const s = this.settings.value;
    return findLeaf(s.layout, s.activePaneId) ?? firstLeaf(s.layout);
  }

  get paneCount(): number {
    return countLeaves(this.layoutRoot);
  }

  /** All tabs open across every pane. */
  get allTabs(): TabSpec[] {
    return allLeaves(this.layoutRoot).flatMap((l) => l.tabs);
  }

  get leftSidebarOpen(): boolean {
    return this.settings.value.leftSidebarOpen;
  }

  get leftSidebarWidth(): number {
    return this.settings.value.leftSidebarWidth;
  }

  get leftSidebarView(): LeftSidebarView {
    return this.settings.value.leftSidebarView;
  }

  get rightSidebarOpen(): boolean {
    return this.settings.value.rightSidebarOpen;
  }

  get rightSidebarWidth(): number {
    return this.settings.value.rightSidebarWidth;
  }

  get activeInspectorTab(): string | undefined {
    return this.settings.value.activeInspectorTab;
  }

  get openInspectorTabs(): string[] | undefined {
    return this.settings.value.openInspectorTabs;
  }

  toggleLeftSidebar(open?: boolean): void {
    this.settings.update({ leftSidebarOpen: open ?? !this.leftSidebarOpen });
  }

  setLeftSidebarWidth(width: number): void {
    this.settings.update({ leftSidebarWidth: Math.round(clamp(width, LEFT_MIN, LEFT_MAX)) });
    this.applyLayoutStyles();
  }

  setLeftSidebarView(view: LeftSidebarView): void {
    this.settings.update({ leftSidebarView: view });
  }

  toggleRightSidebar(open?: boolean): void {
    this.settings.update({ rightSidebarOpen: open ?? !this.rightSidebarOpen });
  }

  setRightSidebarWidth(width: number): void {
    this.settings.update({ rightSidebarWidth: Math.round(clamp(width, RIGHT_MIN, RIGHT_MAX)) });
    this.applyLayoutStyles();
  }

  /** Opens an inspector plugin tab (adding it to the open tabs if absent, or focusing it
   * if already open) and ensures the right sidebar is visible. */
  openInspectorPlugin(key: string): void {
    const current = this.openInspectorTabs ?? [];
    const next = current.includes(key) ? current : [...current, key];
    this.settings.update({ rightSidebarOpen: true, openInspectorTabs: next, activeInspectorTab: key });
  }

  selectInspectorTab(key: string): void {
    this.openInspectorPlugin(key);
  }

  /** Removes a tab from the right sidebar's open tabs. */
  closeInspectorTab(key: string, currentKeys?: string[]): void {
    const base = this.openInspectorTabs ?? currentKeys ?? [];
    const next = base.filter((k) => k !== key);
    const activeInspectorTab = this.activeInspectorTab === key ? next[next.length - 1] : this.activeInspectorTab;
    this.settings.update(
      activeInspectorTab !== undefined
        ? { openInspectorTabs: next, activeInspectorTab }
        : { openInspectorTabs: next, activeInspectorTab: '' },
    );
  }

  #commitLayout(layout: LayoutNode, activePaneId = this.activePaneId): void {
    const active = findLeaf(layout, activePaneId) ?? firstLeaf(layout);
    this.settings.update({
      layout,
      activePaneId: active.id,
      tabs: active.tabs,
      activeTab: active.activeTab,
    });
    this.applyLayoutStyles();
  }

  focusPane(paneId: string): void {
    const leaf = findLeaf(this.layoutRoot, paneId);
    if (!leaf) return;
    if (this.settings.value.activePaneId === leaf.id && this.main.kind === 'tab') return;
    this.#commitLayout(this.layoutRoot, leaf.id);
  }

  splitPane(direction: SplitDirection, paneId: string = this.activePaneId): void {
    const { root, newPaneId } = splitLeaf(this.layoutRoot, paneId, direction);
    this.#commitLayout(root, newPaneId);
    this.main = { kind: 'tab' };
  }

  closePane(paneId: string): void {
    if (this.paneCount <= 1) return;
    const { root, nextActiveId } = closeLeaf(this.layoutRoot, paneId);
    this.#commitLayout(root, paneId === this.activePaneId ? nextActiveId : this.activePaneId);
  }

  resizeSplit(splitId: string, ratio: number): void {
    this.#commitLayout(setSplitRatio(this.layoutRoot, splitId, ratio));
  }

  // ---- tabs (active pane by default) ----

  get tabs(): TabSpec[] {
    return this.activePane.tabs;
  }

  get activeIndex(): number {
    return clampTabIndex(this.tabs, this.activePane.activeTab);
  }

  get activeTab(): TabSpec | undefined {
    return this.tabs[this.activeIndex];
  }

  /** The open thread of the active tab. */
  get activeThread(): ThreadAddress | undefined {
    const t = this.activeTab;
    return t?.thread ? { connection: t.connection, workspace: t.workspace, thread: t.thread } : undefined;
  }

  /** Opens (or focuses) the tab of a workspace on a core in a specific pane (active pane by default). */
  openTab(connection: string, workspace: string, thread?: string, paneId: string = this.activePaneId): void {
    const target = findLeaf(this.layoutRoot, paneId) ?? this.activePane;
    const tabs = [...target.tabs];
    let i = tabs.findIndex((t) => t.connection === connection && t.workspace === workspace);
    if (i < 0) {
      tabs.push({ connection, workspace });
      i = tabs.length - 1;
      this.#watch(connection, workspace);
    }
    if (thread) tabs[i] = { connection, workspace, thread };
    const next = updateLeaf(this.layoutRoot, target.id, (leaf) => ({ ...leaf, tabs, activeTab: i }));
    this.#commitLayout(next, target.id);
    this.main = { kind: 'tab' };
  }

  /** Clears a thread from every pane tab showing it (for example after archiving or deleting). */
  #clearThreadFromTabs(connection: string, workspace: string, threadId: string): void {
    let changed = false;
    const walk = (n: LayoutNode): LayoutNode => {
      if (n.kind === 'split') {
        const first = walk(n.first);
        const second = walk(n.second);
        return first === n.first && second === n.second ? n : { ...n, first, second };
      }
      const tabs = n.tabs.map((t) => {
        if (t.connection === connection && t.workspace === workspace && t.thread === threadId) {
          changed = true;
          return { connection: t.connection, workspace: t.workspace };
        }
        return t;
      });
      return changed ? { ...n, tabs } : n;
    };
    const next = walk(this.layoutRoot);
    if (changed) this.#commitLayout(next, this.activePaneId);
  }

  /** Archives a thread, moving it out of its workspace into the sidebar's Archived section. */
  async archiveThread(connection: string, workspace: string, threadId: string): Promise<boolean> {
    const c = this.connectionFor(connection);
    if (!c) return false;
    const ok = await this.attempt('Cannot archive thread', () => c.call('thread.archive', { thread: threadId }));
    if (!ok) return false;
    const existing = (c.threads[workspace] ?? []).find((t) => t.id === threadId);
    if (existing) c.noteThread({ ...existing, status: 'archived' });
    this.#clearThreadFromTabs(connection, workspace, threadId);
    this.notify('Thread archived', 'info');
    return true;
  }

  /** Restores an archived thread back into its parent workspace. */
  async unarchiveThread(connection: string, workspace: string, threadId: string): Promise<boolean> {
    const c = this.connectionFor(connection);
    if (!c) return false;
    const res = await this.attempt('Cannot unarchive thread', () => c.call('thread.unarchive', { thread: threadId }));
    if (!res) return false;
    c.noteThread(res.thread);
    this.notify('Thread restored', 'success');
    return true;
  }

  /** Opens an in-app confirmation dialog. */
  confirm(options: {
    title: string;
    message: string;
    confirmLabel: string;
    tone?: 'danger' | 'primary';
    onConfirm: () => Promise<void> | void;
  }): void {
    this.dialog = { kind: 'confirm', ...options };
  }

  /** Opens the rename dialog for a thread, prefilled with its current title. */
  renameThread(address: ThreadAddress, title: string): void {
    this.dialog = { kind: 'rename-thread', address, title };
  }

  /**
   * Saves a thread's title as the user's own: the agent's suggestions no longer replace
   * it (ADR 0030). Returns `true` when it was saved.
   */
  async saveThreadTitle(address: ThreadAddress, title: string): Promise<boolean> {
    const trimmed = title.trim();
    if (!trimmed) return false;
    const saved = await this.attempt('Cannot rename thread', () =>
      this.threads.update(address, { title: trimmed }),
    );
    if (!saved) return false;
    this.notify('Thread renamed', 'success');
    return true;
  }

  /** Removes every tab of a deleted workspace across all split panes, closing empty secondary panes. */
  #removeWorkspaceFromTabs(connection: string, workspace: string): void {
    let changed = false;
    const walk = (n: LayoutNode): LayoutNode => {
      if (n.kind === 'split') {
        const first = walk(n.first);
        const second = walk(n.second);
        return first === n.first && second === n.second ? n : { ...n, first, second };
      }
      const tabs = n.tabs.filter((t) => !(t.connection === connection && t.workspace === workspace));
      if (tabs.length === n.tabs.length) return n;
      changed = true;
      return { ...n, tabs, activeTab: clampTabIndex(tabs, n.activeTab) };
    };
    let root = walk(this.layoutRoot);
    if (!changed) return;
    let activeId = this.activePaneId;
    for (const leaf of allLeaves(root)) {
      if (leaf.tabs.length === 0 && allLeaves(root).length > 1) {
        const closed = closeLeaf(root, leaf.id);
        root = closed.root;
        if (activeId === leaf.id) activeId = closed.nextActiveId;
      }
    }
    this.#commitLayout(root, activeId);
  }

  /** Permanently deletes a workspace and all its threads from Kumikit, closing its tabs. */
  async deleteWorkspace(connection: string, workspace: string): Promise<boolean> {
    const c = this.connectionFor(connection);
    if (!c) return false;
    const isChat = c.workspaces.find((w) => w.id === workspace)?.kind === 'chat';
    const ok = await this.attempt(isChat ? 'Cannot delete chats' : 'Cannot delete project', () =>
      c.call('workspace.delete', { workspace }),
    );
    if (!ok) return false;
    const key = `${connection}/${workspace}`;
    const stop = this.#unwatch.get(key);
    if (stop) {
      stop();
      this.#unwatch.delete(key);
    }
    c.removeWorkspace(workspace);
    this.#removeWorkspaceFromTabs(connection, workspace);
    this.notify(isChat ? 'Chats deleted' : 'Project deleted', 'info');
    return true;
  }

  /** Permanently deletes a thread and removes it from all open panes. */
  async deleteThread(connection: string, workspace: string, threadId: string): Promise<boolean> {
    const c = this.connectionFor(connection);
    if (!c) return false;
    const ok = await this.attempt('Cannot delete thread', () => c.call('thread.delete', { thread: threadId }));
    if (!ok) return false;
    c.removeThread(workspace, threadId);
    this.#clearThreadFromTabs(connection, workspace, threadId);
    this.notify('Thread deleted', 'info');
    return true;
  }

  closeTab(index: number, paneId: string = this.activePaneId): void {
    const target = findLeaf(this.layoutRoot, paneId) ?? this.activePane;
    const tabs = target.tabs.filter((_, i) => i !== index);
    if (tabs.length === 0 && this.paneCount > 1) {
      this.closePane(target.id);
      return;
    }
    const activeTab = clampTabIndex(tabs, target.activeTab);
    const next = updateLeaf(this.layoutRoot, target.id, (leaf) => ({ ...leaf, tabs, activeTab }));
    this.#commitLayout(next, this.activePaneId);
  }

  selectTab(index: number, paneId: string = this.activePaneId): void {
    const target = findLeaf(this.layoutRoot, paneId) ?? this.activePane;
    const activeTab = clampTabIndex(target.tabs, index);
    const next = updateLeaf(this.layoutRoot, target.id, (leaf) => ({ ...leaf, activeTab }));
    this.#commitLayout(next, target.id);
    this.main = { kind: 'tab' };
  }

  #watch(connection: string, workspace: string): void {
    const key = `${connection}/${workspace}`;
    const c = this.connectionFor(connection);
    if (c && !this.#unwatch.has(key)) this.#unwatch.set(key, c.watchWorkspace(workspace));
  }

  /** A connection as the UI SDK shows it to plugins: the active tab's by default.
   * Inline plugins run in the page, so this is not a security boundary; the limits for
   * sandboxed panels come with them (M4). */
  pluginConnection(id?: string): Connection {
    const c = this.connectionFor(id) ?? this.connection;
    return {
      id: c.id,
      get ready() {
        return c.ready;
      },
      call: (method, params) => c.call(method, params),
      subscribe: (topic, after, fn) => c.subscribe(topic, after, fn),
      models: (provider) => c.models(provider),
    };
  }

  // ---- actions ----

  notify(message: string, tone: Tone = 'info'): void {
    const toast = { id: ++this.#toastId, message, tone };
    this.toasts = [...this.toasts.slice(-4), toast];
  }

  dismiss(id: number): void {
    this.toasts = this.toasts.filter((t) => t.id !== id);
  }

  /** Runs an action and shows its failure instead of throwing. */
  async attempt<T>(what: string, f: () => Promise<T>): Promise<T | undefined> {
    try {
      return await f();
    } catch (e) {
      const message = e instanceof RpcError || e instanceof Error ? e.message : String(e);
      this.notify(`${what}: ${message}`, 'danger');
      return undefined;
    }
  }

  /** The first-run welcome's "Browse the starter pack": answers the welcome and shows
   * the registry's starter pack (the first time the registries are contacted). */
  openStarterPack(): void {
    this.settings.update({ welcomeDone: true });
    this.main = { kind: 'plugins', starter: true };
    void this.registries.search('');
  }

  /** The first-run welcome's "Not now": remembered; the palette still offers it. */
  dismissWelcome(): void {
    this.settings.update({ welcomeDone: true });
  }

  /** Opens a project folder on a core (the active tab's by default). Only the local core's
   * folders can be picked; other cores take a path on their machine, typed by the user. */
  async openWorkspaceFolder(connection: string = this.connection.id, typed?: string): Promise<void> {
    const c = this.connectionFor(connection);
    if (!c) return;
    const path = typed ?? (c.kind === 'local' ? await this.platform.pickFolder('Open a project folder') : null);
    if (!path) {
      if (c.kind !== 'local' && typed === undefined) this.dialog = { kind: 'open-folder', connection };
      return;
    }
    const ws = await this.attempt('Cannot open the folder', () => c.openWorkspace(path));
    if (ws) this.openTab(connection, ws.id);
  }

  /** Starts a new standalone chat without requiring a project folder. */
  async startNewChat(connection: string = this.connection.id, paneId: string = this.activePaneId): Promise<void> {
    const c = this.connectionFor(connection);
    if (!c || !c.ready) return;
    const existing = c.workspaces.find((w) => w.kind === 'chat');
    const ws = existing ?? (await this.attempt('Cannot start chat', () => c.openChatWorkspace()));
    if (ws) this.startDraftThread({ connection: c.id, workspace: ws.id }, paneId);
  }

  /** Opens the native multi-file attachment dialog rooted at the workspace (or thread worktree)
   * folder on the desktop shell, or returns `null` when the platform should fall back to `<input type="file">`. */
  async pickAttachments(connection: string, workspace: string, worktreePath?: string): Promise<PickedAttachment[] | null> {
    if (!this.platform.pickAttachments) return null;
    const conn = this.connectionFor(connection);
    const root =
      conn?.kind === 'local'
        ? (worktreePath ?? conn.workspaces.find((w) => w.id === workspace)?.root)
        : undefined;
    const picked = await this.attempt('Cannot attach files', () =>
      this.platform.pickAttachments!('Attach files', root),
    );
    return picked ?? [];
  }

  /** Starts a fresh draft thread in a workspace tab (clears its open thread so the inline
   * new-thread composer appears, with no modal dialog). Optionally pins the draft to a
   * linked Git worktree path while keeping the thread under the parent workspace. */
  startDraftThread(
    target?: { connection: string; workspace: string; worktreePath?: string },
    paneId: string = this.activePaneId,
  ): void {
    const dest = target ?? this.activeTab;
    if (!dest) return;
    const leaf = findLeaf(this.layoutRoot, paneId) ?? this.activePane;
    const tabs = [...leaf.tabs];
    const draftTab: TabSpec = target?.worktreePath
      ? { connection: dest.connection, workspace: dest.workspace, worktreePath: target.worktreePath }
      : { connection: dest.connection, workspace: dest.workspace };
    let i = tabs.findIndex((t) => t.connection === dest.connection && t.workspace === dest.workspace);
    if (i < 0) {
      tabs.push(draftTab);
      i = tabs.length - 1;
      this.#watch(dest.connection, dest.workspace);
    } else {
      tabs[i] = draftTab;
    }
    const next = updateLeaf(this.layoutRoot, leaf.id, (l) => ({ ...l, tabs, activeTab: i }));
    this.#commitLayout(next, leaf.id);
    this.main = { kind: 'tab' };
    this.draftFocusNonce++;
  }

  async createThread(
    provider: string,
    model: string | undefined,
    title: string | undefined,
    target?: { connection: string; workspace: string },
    paneId: string = this.activePaneId,
    permissionMode?: PermissionMode,
    worktreePath?: string,
    options?: Record<string, string>,
  ): Promise<Thread | undefined> {
    const tab = target ?? this.activeTab;
    if (!tab) return undefined;
    const params: {
      workspace: string;
      provider: string;
      model?: string;
      title?: string;
      permissionMode?: PermissionMode;
      worktreePath?: string;
      options?: Record<string, string>;
    } = { workspace: tab.workspace, provider };
    if (model) params.model = model;
    if (options && Object.keys(options).length > 0) params.options = options;
    if (title) params.title = title;
    if (permissionMode && permissionMode !== 'standard') params.permissionMode = permissionMode;
    if (worktreePath) params.worktreePath = worktreePath;
    const c = this.connectionFor(tab.connection);
    if (!c) return undefined;
    const r = await this.attempt('Cannot start the thread', () => c.call('thread.create', params));
    if (!r) return undefined;
    const prefs: { lastProvider: string; lastModel?: string; lastOptions?: Record<string, string> } = {
      lastProvider: provider,
      lastOptions: { ...(options ?? {}) },
    };
    if (model) prefs.lastModel = model;
    this.settings.update(prefs);
    this.noteRecentModel(tab.connection, provider, r.thread.model);
    c.noteThread(r.thread);
    this.openTab(tab.connection, tab.workspace, r.thread.id, paneId);
    return r.thread;
  }

  /** Records `provider`/`model` as the most recent pick on `connection` (model picker). */
  noteRecentModel(connection: string, provider: string, model: string): void {
    const all = this.settings.value.recentModels ?? {};
    this.settings.update({ recentModels: { ...all, [connection]: pushRecent(all[connection] ?? [], { provider, model }) } });
  }

  /** Creates a new thread in `paneId` and sends the opening message in one seamless step. */
  async startThreadWithMessage(
    connection: string,
    workspace: string,
    provider: string,
    model: string | undefined,
    text: string,
    paneId: string = this.activePaneId,
    options?: {
      permissionMode?: PermissionMode;
      attachments?: Attachment[];
      worktreePath?: string;
      modelOptions?: Record<string, string>;
    },
  ): Promise<Thread | undefined> {
    const firstLine = text.split(/\r?\n/)[0]?.trim() ?? '';
    const fallbackTitle = options?.attachments?.[0]?.name;
    const rawTitle = firstLine || fallbackTitle || '';
    const title = rawTitle.length > 60 ? `${rawTitle.slice(0, 57)}…` : rawTitle || undefined;
    const thread = await this.createThread(
      provider,
      model,
      title,
      { connection, workspace },
      paneId,
      options?.permissionMode,
      options?.worktreePath,
      options?.modelOptions,
    );
    if (!thread) return undefined;
    await this.attempt('Cannot send', () =>
      this.threads.send({ connection, workspace, thread: thread.id }, text, options?.attachments),
    );
    return thread;
  }

  #bundledThemes: ThemeVariant[] = [];

  /** Loads bundled themes from `./themes/index.json` and merges user-imported custom themes. */
  async loadThemes(): Promise<void> {
    const loaded: ThemeVariant[] = [];
    try {
      const r = await fetch('./themes/index.json');
      if (r.ok) {
        const list = (await r.json()) as unknown;
        if (Array.isArray(list)) {
          for (const raw of list) {
            try {
              const parsed = parseThemeObject(raw);
              loaded.push({ ...parsed.theme, source: 'bundled' });
            } catch {
              // Skip malformed bundled theme entry.
            }
          }
        }
      }
    } catch {
      // No themes/index.json (development or unit tests without a bundle).
    }
    this.#bundledThemes = loaded;
    this.#syncThemes();
  }

  #syncThemes(): void {
    const custom: ThemeVariant[] = this.settings.value.customThemes.map((t) => ({
      id: t.id,
      name: t.name,
      appearance: t.appearance,
      source: 'custom',
      tokens: t.tokens,
    }));
    this.theme.setVariants([...this.#bundledThemes, ...custom]);
    this.refreshThemeCommands();
    this.applyTheme();
  }

  /** Prompts for a `.json` theme file on disk, validates and saves it, and selects it. */
  async importThemeFromFile(): Promise<import('@kumikit/ui-sdk').ThemeVariantInfo | undefined> {
    const picked = await this.platform.pickBytes('.json,application/json');
    if (!picked) return undefined;
    const text = new TextDecoder().decode(picked.bytes);
    return this.importThemeText(picked.name, text);
  }

  /** Validates and imports a theme from raw JSON text, saving it in client settings. */
  importThemeText(fileName: string, text: string): import('@kumikit/ui-sdk').ThemeVariantInfo | undefined {
    let parsed: ReturnType<typeof parseThemeFile>;
    try {
      parsed = parseThemeFile(text, fileName);
    } catch (e) {
      this.notify(`Cannot import theme: ${e instanceof Error ? e.message : String(e)}`, 'danger');
      return undefined;
    }
    const id = `custom:${parsed.theme.id}`;
    const entry = {
      id,
      name: parsed.theme.name,
      appearance: parsed.theme.appearance,
      tokens: parsed.theme.tokens,
    };
    const rest = this.settings.value.customThemes.filter((t) => t.id !== id);
    this.settings.update({ customThemes: [...rest, entry], themeVariant: id });
    this.#syncThemes();
    if (parsed.errors.length > 0) {
      this.notify(
        `Theme ${entry.name}: ${parsed.errors.length} token(s) ignored (${parsed.errors.slice(0, 3).join('; ')})`,
        'warning',
      );
    }
    if (parsed.warnings.length > 0) {
      this.notify(
        `${entry.name} has low contrast in places (${parsed.warnings.length}): ${parsed.warnings.slice(0, 2).join('; ')}`,
        'warning',
      );
    }
    this.notify(`Added theme ${entry.name}`, 'success');
    const variant = this.theme.variants.find((v) => v.id === id);
    return variant
      ? {
          key: variant.id,
          name: variant.name,
          appearance: variant.appearance,
          source: variant.source,
          preview: this.theme.previewTokens(variant),
        }
      : undefined;
  }

  /** Removes a user-imported theme from client settings. */
  removeCustomTheme(key: string): void {
    const existing = this.settings.value.customThemes.find((t) => t.id === key);
    if (!existing) return;
    const next = this.settings.value.customThemes.filter((t) => t.id !== key);
    const change: Partial<typeof this.settings.value> = { customThemes: next };
    if (this.settings.value.themeVariant === key) change.themeVariant = undefined;
    this.settings.update(change);
    this.#syncThemes();
    this.notify(`Removed theme ${existing.name}`, 'info');
  }

  applyTheme(): void {
    const s = this.settings.value;
    const choice: { variant?: string; appearance: typeof s.appearance; reduceMotion: boolean } = {
      appearance: s.appearance,
      reduceMotion: s.reduceMotion,
    };
    if (s.themeVariant) choice.variant = s.themeVariant;
    this.theme.apply(choice, this.#systemDark());
  }

  selectTheme(variant: string | undefined, appearance?: 'system' | 'light' | 'dark'): void {
    const change: Partial<typeof this.settings.value> = {};
    if (appearance) change.appearance = appearance;
    this.settings.update({ ...change, themeVariant: variant } as Partial<typeof this.settings.value>);
    this.applyTheme();
  }

  /** Quick 1-click toggle between light and dark appearances. */
  toggleAppearance(): void {
    const next = this.theme.appearance === 'dark' ? 'light' : 'dark';
    this.selectTheme(undefined, next);
  }

  /** Context for `when` clauses. */
  get when(): WhenContext {
    return {
      'connection.ready': this.connection.ready,
      'tab.open': this.activeTab !== undefined,
      'thread.open': this.activeThread !== undefined,
      'pane.split': this.paneCount > 1,
      // Restarting is offered for the local core only: stopping a core elsewhere would
      // not start it again.
      'core.mismatch': this.connection.status === 'mismatch' && this.connection.kind === 'local',
      'core.down': this.connection.status === 'reconnecting' || this.connection.status === 'connecting',
    };
  }

  #registerCommands(): void {
    const c = this.commands;
    c.register({ id: 'kernel.palette', title: 'Show all commands', keybinding: 'Mod+K' }, () => {
      this.dialog = { kind: 'palette' };
    });
    c.register({ id: 'workspace.open', title: 'Open a project folder…', keybinding: 'Mod+O', when: 'connection.ready' }, () =>
      this.openWorkspaceFolder(),
    );
    c.register({ id: 'chat.new', title: 'New chat', keybinding: 'Mod+Shift+N', when: 'connection.ready' }, () =>
      this.startNewChat(),
    );
    c.register({ id: 'thread.new', title: 'New thread', keybinding: 'Mod+N', when: 'connection.ready && tab.open' }, (args) => {
      const a = args as { connection?: string; workspace?: string; worktreePath?: string } | undefined;
      if (a?.connection && a?.workspace) {
        const target: { connection: string; workspace: string; worktreePath?: string } = {
          connection: a.connection,
          workspace: a.workspace,
        };
        if (a.worktreePath) target.worktreePath = a.worktreePath;
        this.startDraftThread(target);
      } else {
        this.startDraftThread();
      }
    });
    c.register({ id: 'thread.cancel', title: 'Cancel the running turn', when: 'thread.open' }, async () => {
      const t = this.activeThread;
      if (t) await this.attempt('Cannot cancel', () => this.threads.cancel(t));
    });
    c.register({ id: 'tab.close', title: 'Close tab', keybinding: 'Mod+W', when: 'tab.open' }, () => this.closeTab(this.activeIndex));
    c.register({ id: 'tab.next', title: 'Next tab', keybinding: 'Ctrl+Tab', when: 'tab.open' }, () =>
      this.selectTab((this.activeIndex + 1) % Math.max(this.tabs.length, 1)),
    );
    c.register({ id: 'layout.toggleLeftSidebar', title: 'Toggle left sidebar', keybinding: 'Mod+B' }, () => this.toggleLeftSidebar());
    c.register({ id: 'layout.toggleRightSidebar', title: 'Toggle right inspector', keybinding: 'Mod+Shift+B' }, () => this.toggleRightSidebar());
    c.register({ id: 'layout.splitRight', title: 'Split pane right', keybinding: 'Mod+\\' }, () => this.splitPane('horizontal'));
    c.register({ id: 'layout.splitDown', title: 'Split pane down', keybinding: 'Mod+Shift+\\' }, () => this.splitPane('vertical'));
    c.register({ id: 'layout.closePane', title: 'Close split pane', when: 'pane.split' }, () => this.closePane(this.activePaneId));
    c.register({ id: 'settings.open', title: 'Open settings', keybinding: 'Mod+,' }, () => {
      this.main = { kind: 'settings' };
    });
    c.register({ id: 'agents.open', title: 'Agents: install, remove and configure' }, () => {
      this.main = { kind: 'settings', page: 'agents' };
    });
    c.register({ id: 'plugins.open', title: 'Manage plugins' }, () => {
      this.main = { kind: 'plugins' };
    });
    c.register({ id: 'plugins.starter', title: 'Starter pack: common plugins from the registry' }, () => this.openStarterPack());
    c.register({ id: 'plugins.updates', title: 'Check for plugin updates' }, async () => {
      this.main = { kind: 'plugins' };
      await this.registries.checkUpdates();
    });
    const browser = this.platform.browser;
    if (browser) {
      c.register({ id: 'core.signOut', title: 'Sign out of this core' }, async () => {
        await this.attempt('Cannot sign out', () => browser.signOut());
        this.connection.reconnectNow();
      });
    }
    c.register({ id: 'theme.system', title: 'Theme: follow the system' }, () => this.selectTheme(undefined, 'system'));
    c.register({ id: 'theme.toggle', title: 'Theme: toggle light and dark' }, () => this.toggleAppearance());
    c.register({ id: 'cores.open', title: 'Cores: add, remove, reconnect' }, () => {
      this.main = { kind: 'cores' };
    });
    c.register({ id: 'core.reconnect', title: 'Reconnect to this core now', when: 'core.down' }, () => this.connection.reconnectNow());
    c.register({ id: 'core.restart', title: 'Restart the core', when: 'core.mismatch' }, async () => {
      const ok = await this.connection.restartMismatchedCore();
      if (ok) await this.connection.start();
      else this.notify('This core cannot be restarted from here. Stop the running `kumikit serve` and reopen the window.', 'warning');
    });
    c.register({ id: 'approval.respond', title: 'Answer an approval', when: 'never' }, async (args) => {
      // The approval is on the active tab's core, unless the caller names the thread.
      const a = args as { approval: string; decision: 'allow' | 'allow_always' | 'deny'; address?: ThreadAddress };
      const connection = a.address?.connection ?? this.connection.id;
      await this.attempt('Cannot answer the approval', () => this.threads.respond(connection, a.approval, a.decision));
    });
    c.register({ id: 'thread.send', title: 'Send a message', when: 'never' }, async (args) => {
      const a = args as { address: ThreadAddress; text: string; attachments?: Attachment[] };
      await this.attempt('Cannot send', () => this.threads.send(a.address, a.text, a.attachments));
    });
    c.register({ id: 'thread.steer', title: 'Steer the running turn', when: 'never' }, async (args) => {
      const a = args as { address: ThreadAddress; text: string };
      await this.attempt('Cannot steer', async () => {
        const status = await this.threads.steer(a.address, a.text);
        if (status === 'queued') this.notify('This agent cannot be steered; the message is first in the queue.', 'info');
      });
    });
    type QueueArgs = { address: ThreadAddress; entry: string };
    c.register({ id: 'thread.queue.update', title: 'Edit a queued message', when: 'never' }, async (args) => {
      const a = args as QueueArgs & { text: string };
      await this.attempt('Cannot edit the queued message', () => this.threads.queueUpdate(a.address, a.entry, a.text));
    });
    c.register({ id: 'thread.queue.remove', title: 'Remove a queued message', when: 'never' }, async (args) => {
      const a = args as QueueArgs;
      await this.attempt('Cannot remove the queued message', () => this.threads.queueRemove(a.address, a.entry));
    });
    c.register({ id: 'thread.queue.move', title: 'Reorder the queue', when: 'never' }, async (args) => {
      const a = args as QueueArgs & { before?: string };
      await this.attempt('Cannot reorder the queue', () => this.threads.queueMove(a.address, a.entry, a.before));
    });
    c.register({ id: 'thread.queue.sendNow', title: 'Send a queued message now', when: 'never' }, async (args) => {
      const a = args as QueueArgs;
      await this.attempt('Cannot send the queued message', () => this.threads.queueSendNow(a.address, a.entry));
    });
    c.register({ id: 'thread.queue.steer', title: 'Steer with a queued message', when: 'never' }, async (args) => {
      const a = args as QueueArgs;
      await this.attempt('Cannot steer', async () => {
        const status = await this.threads.queueSteer(a.address, a.entry);
        if (status === 'queued') this.notify('This agent cannot be steered; the message is first in the queue.', 'info');
      });
    });
    c.register({ id: 'thread.queue.resume', title: 'Resume the queue', when: 'never' }, async (args) => {
      const a = args as { address: ThreadAddress };
      await this.attempt('Cannot resume the queue', () => this.threads.queueResume(a.address));
    });
    c.register({ id: 'thread.defer', title: 'Schedule a deferred message', when: 'never' }, async (args) => {
      const a = args as {
        address: ThreadAddress;
        text: string;
        trigger: import('@kumikit/protocol').QueueTrigger;
        attachments?: Attachment[];
      };
      await this.attempt('Cannot schedule message', () =>
        this.threads.defer(a.address, a.text, a.trigger, a.attachments),
      );
    });
    c.register({ id: 'thread.update', title: 'Update thread settings', when: 'never' }, async (args) => {
      const a = args as {
        address: ThreadAddress;
        provider?: string;
        model?: string;
        permissionMode?: PermissionMode;
        title?: string;
        handoff?: import('@kumikit/protocol').HandoffMode;
      };
      const patch: {
        provider?: string;
        model?: string;
        permissionMode?: PermissionMode;
        title?: string;
        handoff?: import('@kumikit/protocol').HandoffMode;
      } = {};
      if (a.provider !== undefined) patch.provider = a.provider;
      if (a.model !== undefined) patch.model = a.model;
      if (a.permissionMode !== undefined) patch.permissionMode = a.permissionMode;
      if (a.title !== undefined) patch.title = a.title;
      if (a.handoff !== undefined) patch.handoff = a.handoff;
      await this.attempt('Cannot update thread', () => this.threads.update(a.address, patch));
    });
    c.register({ id: 'thread.open', title: 'Open a thread', when: 'never' }, (args) => {
      const a = args as { address: ThreadAddress };
      this.openTab(a.address.connection, a.address.workspace, a.address.thread);
    });
    c.register({ id: 'thread.fork', title: 'Fork a thread', when: 'never' }, async (args) => {
      const a = args as {
        address: ThreadAddress;
        atTurn?: string;
        beforeTurn?: string;
        title?: string;
        open?: boolean;
      };
      const opts: { atTurn?: string; beforeTurn?: string; title?: string } = {};
      if (a.atTurn !== undefined) opts.atTurn = a.atTurn;
      if (a.beforeTurn !== undefined) opts.beforeTurn = a.beforeTurn;
      if (a.title !== undefined) opts.title = a.title;
      const forked = await this.attempt('Cannot fork thread', () => this.threads.fork(a.address, opts));
      if (forked && a.open !== false) {
        this.openTab(a.address.connection, a.address.workspace, forked.id);
      }
      return forked;
    });
    c.register({ id: 'inspector.open', title: 'Open inspector panel', when: 'never' }, (args) => {
      const a = args as { tab: string };
      if (a?.tab) this.openInspectorPlugin(a.tab);
    });
    c.register({ id: 'composer.insert', title: 'Insert text into composer', when: 'never' }, (args) => {
      const a = args as { address?: ThreadAddress; text: string };
      if (typeof window !== 'undefined' && a?.text) {
        window.dispatchEvent(new CustomEvent('kumikit:composer-insert', { detail: a }));
      }
    });
    c.register({ id: 'attachment.pick', title: 'Attach files', when: 'never' }, async (args) => {
      const a = args as { address?: ThreadAddress; connection?: string; workspace?: string; worktreePath?: string } | undefined;
      const connection = a?.address?.connection ?? a?.connection ?? this.activeTab?.connection ?? this.connection.id;
      const workspace = a?.address?.workspace ?? a?.workspace ?? this.activeTab?.workspace ?? '';
      return this.pickAttachments(connection, workspace, a?.worktreePath);
    });
    if (this.updates.supported) {
      c.register({ id: 'app.updates.check', title: 'Check for updates' }, () => {
        this.main = { kind: 'settings', page: 'updates' };
        void this.updates.checkNow();
      });
    }
  }

  /** One command per theme variant; called when the installed themes change. */
  #themeCommands: (() => void)[] = [];
  refreshThemeCommands(): void {
    for (const d of this.#themeCommands) d();
    this.#themeCommands = this.theme.variants.map((v) =>
      this.commands.register({ id: `theme.select:${variantKey(v)}`, title: `Theme: ${v.name}` }, () => this.selectTheme(variantKey(v))),
    );
  }
}
