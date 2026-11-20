/**
 * The UI kernel's state: the connections (one per core), tabs, threads, commands, slots,
 * theme and settings, and the kernel's own commands. Components read it; nothing here
 * names a plugin.
 */
import type { Thread } from '@kumikit/protocol';
import type { Connection, Slot, ThreadAddress } from '@kumikit/ui-sdk';

import { CommandRegistry, type WhenContext } from './commands/registry.svelte';
import { CoreCommands, type CoreView } from './commands/core.svelte';
import { ClientPlugins } from './plugins/client.svelte';
import { CorePlugins } from './plugins/core.svelte';
import { Registries } from './plugins/registry.svelte';
import { CoreConnection, type Timers } from './connections/connection.svelte';
import type { Platform, ProfileInfo } from './platform/index';
import { meetsRequirements } from './plugins/requires';
import type { Command } from './commands/registry.svelte';
import type { Entry } from './slots/registry.svelte';
import { RpcError } from './rpc/client';
import { SettingsStore, type TabSpec } from './settings/settings.svelte';
import { SlotRegistry } from './slots/registry.svelte';
import { ThemeEngine, variantKey } from './theme/engine.svelte';
import { ThreadStore } from './threads/store.svelte';

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
  | { kind: 'new-thread' }
  | { kind: 'palette' }
  | { kind: 'open-folder'; connection: string }
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
  main = $state.raw<MainView>({ kind: 'tab' });
  /** Core plugins' commands for the active tab's core. */
  readonly coreCommands: CoreCommands;
  dialog = $state.raw<Dialog>(undefined);
  toasts = $state.raw<Toast[]>([]);
  #toastId = 0;
  #systemDark: () => boolean;
  #unwatch = new Map<string, () => void>();
  #timers: Timers | undefined;

  constructor(options: KernelOptions) {
    this.platform = options.platform;
    this.mac = options.mac ?? false;
    this.#timers = options.timers;
    this.connections = [new CoreConnection(options.platform.home, options.platform, this.#options(homeProfile(options.platform)))];
    this.threads = new ThreadStore((id) => this.connectionFor(id));
    this.slots = new SlotRegistry((plugin, reason) => this.onPluginFailed(plugin, reason));
    this.theme = new ThemeEngine(options.document);
    this.settings = new SettingsStore(options.storage);
    this.#systemDark = options.systemDark ?? (() => false);
    this.clientPlugins = new ClientPlugins(this);
    this.corePlugins = new CorePlugins(this);
    this.registries = new Registries(this);
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
    // Plugins and every core start side by side; none waits for another.
    const plugins = this.clientPlugins.load();
    const profiles = this.platform.profiles.list().catch((e: unknown) => {
      this.notify(`Cannot list the cores: ${e instanceof Error ? e.message : String(e)}`, 'danger');
      return [] as ProfileInfo[];
    });
    const home = this.connections[0]!.start();
    for (const p of await profiles) if (!this.connectionFor(p.name)) this.#add(p);
    for (const tab of this.tabs) this.#watch(tab.connection, tab.workspace);
    await Promise.all([home, plugins]);
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
    const c = new CoreConnection(profile.name, this.platform, this.#options({ label: profile.label, kind: profile.kind }));
    this.connections = [...this.connections, c];
    void c.start();
    return c;
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

  // ---- tabs ----

  get tabs(): TabSpec[] {
    return this.settings.value.tabs;
  }

  get activeIndex(): number {
    return Math.min(this.settings.value.activeTab, Math.max(this.tabs.length - 1, 0));
  }

  get activeTab(): TabSpec | undefined {
    return this.tabs[this.activeIndex];
  }

  /** The open thread of the active tab. */
  get activeThread(): ThreadAddress | undefined {
    const t = this.activeTab;
    return t?.thread ? { connection: t.connection, workspace: t.workspace, thread: t.thread } : undefined;
  }

  /** Opens (or focuses) the tab of a workspace on a core, optionally on a thread. */
  openTab(connection: string, workspace: string, thread?: string): void {
    const tabs = [...this.tabs];
    let i = tabs.findIndex((t) => t.connection === connection && t.workspace === workspace);
    if (i < 0) {
      tabs.push({ connection, workspace });
      i = tabs.length - 1;
      this.#watch(connection, workspace);
    }
    if (thread) tabs[i] = { ...tabs[i]!, thread };
    this.settings.update({ tabs, activeTab: i });
    this.main = { kind: 'tab' };
  }

  closeTab(index: number): void {
    const tabs = this.tabs.filter((_, i) => i !== index);
    this.settings.update({ tabs, activeTab: Math.max(0, Math.min(this.activeIndex, tabs.length - 1)) });
  }

  selectTab(index: number): void {
    this.settings.update({ activeTab: index });
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

  async createThread(provider: string, model: string | undefined, title: string | undefined): Promise<Thread | undefined> {
    const tab = this.activeTab;
    if (!tab) return undefined;
    const params: { workspace: string; provider: string; model?: string; title?: string } = { workspace: tab.workspace, provider };
    if (model) params.model = model;
    if (title) params.title = title;
    const c = this.connectionFor(tab.connection);
    if (!c) return undefined;
    const r = await this.attempt('Cannot start the thread', () => c.call('thread.create', params));
    if (!r) return undefined;
    c.noteThread(r.thread);
    this.openTab(tab.connection, tab.workspace, r.thread.id);
    return r.thread;
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

  /** Context for `when` clauses. */
  get when(): WhenContext {
    return {
      'connection.ready': this.connection.ready,
      'tab.open': this.activeTab !== undefined,
      'thread.open': this.activeThread !== undefined,
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
    c.register({ id: 'thread.new', title: 'New thread…', keybinding: 'Mod+N', when: 'connection.ready && tab.open' }, () => {
      this.dialog = { kind: 'new-thread' };
    });
    c.register({ id: 'thread.cancel', title: 'Cancel the running turn', when: 'thread.open' }, async () => {
      const t = this.activeThread;
      if (t) await this.attempt('Cannot cancel', () => this.threads.cancel(t));
    });
    c.register({ id: 'tab.close', title: 'Close tab', keybinding: 'Mod+W', when: 'tab.open' }, () => this.closeTab(this.activeIndex));
    c.register({ id: 'tab.next', title: 'Next tab', keybinding: 'Ctrl+Tab', when: 'tab.open' }, () =>
      this.selectTab((this.activeIndex + 1) % Math.max(this.tabs.length, 1)),
    );
    c.register({ id: 'settings.open', title: 'Open settings', keybinding: 'Mod+,' }, () => {
      this.main = { kind: 'settings' };
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
      const a = args as { address: ThreadAddress; text: string };
      await this.attempt('Cannot send', () => this.threads.send(a.address, a.text));
    });
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
