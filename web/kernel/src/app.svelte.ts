/**
 * The UI kernel's state: the local connection, tabs, threads, commands, slots, theme
 * and settings, and the kernel's own commands. Components read it; nothing here names a
 * plugin.
 */
import type { Thread } from '@kumikit/protocol';
import type { Connection, ThreadAddress } from '@kumikit/ui-sdk';

import { CommandRegistry, type WhenContext } from './commands/registry.svelte';
import { CoreConnection } from './connections/connection.svelte';
import type { Platform } from './platform/index';
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
  | { kind: 'plugins' }
  | { kind: 'view'; id: string; args?: unknown };

/** A dialog the kernel shows over everything. */
export type Dialog = { kind: 'new-thread' } | { kind: 'palette' } | undefined;

export interface KernelOptions {
  platform: Platform;
  storage?: Storage;
  document?: Document;
  systemDark?: () => boolean;
  mac?: boolean;
}

export class Kernel {
  readonly platform: Platform;
  readonly connection: CoreConnection;
  readonly threads: ThreadStore;
  readonly commands = new CommandRegistry();
  readonly slots: SlotRegistry;
  readonly theme: ThemeEngine;
  readonly settings: SettingsStore;
  readonly mac: boolean;
  main = $state.raw<MainView>({ kind: 'tab' });
  dialog = $state.raw<Dialog>(undefined);
  toasts = $state.raw<Toast[]>([]);
  #toastId = 0;
  #systemDark: () => boolean;
  #unwatch = new Map<string, () => void>();

  constructor(options: KernelOptions) {
    this.platform = options.platform;
    this.mac = options.mac ?? false;
    this.connection = new CoreConnection('local', options.platform);
    this.threads = new ThreadStore(this.connection);
    this.slots = new SlotRegistry((plugin, reason) => this.onPluginFailed(plugin, reason));
    this.theme = new ThemeEngine(options.document);
    this.settings = new SettingsStore(options.storage);
    this.#systemDark = options.systemDark ?? (() => false);
    this.#registerCommands();
  }

  /** Called when the slot registry disables a plugin; the plugin manager replaces it. */
  onPluginFailed = (plugin: string, reason: string): void => {
    this.notify(`The plugin ${plugin} was disabled after repeated failures: ${reason}`, 'danger');
  };

  async start(): Promise<void> {
    this.applyTheme();
    await this.connection.start();
    for (const tab of this.tabs) this.#watch(tab.workspace);
  }

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

  /** Opens (or focuses) the tab of a workspace, optionally on a thread. */
  openTab(workspace: string, thread?: string): void {
    const tabs = [...this.tabs];
    let i = tabs.findIndex((t) => t.connection === this.connection.id && t.workspace === workspace);
    if (i < 0) {
      tabs.push({ connection: this.connection.id, workspace });
      i = tabs.length - 1;
      this.#watch(workspace);
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

  #watch(workspace: string): void {
    if (!this.#unwatch.has(workspace)) this.#unwatch.set(workspace, this.connection.watchWorkspace(workspace));
  }

  /** The active connection as the UI SDK shows it to plugins. Inline plugins run in the
   * page, so this is not a security boundary; the limits for sandboxed panels come with
   * them (M4). */
  pluginConnection(): Connection {
    const c = this.connection;
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

  async openWorkspaceFolder(): Promise<void> {
    const path = await this.platform.pickFolder('Open a project folder');
    if (!path) return;
    const ws = await this.attempt('Cannot open the folder', () => this.connection.openWorkspace(path));
    if (ws) this.openTab(ws.id);
  }

  async createThread(provider: string, model: string | undefined, title: string | undefined): Promise<Thread | undefined> {
    const tab = this.activeTab;
    if (!tab) return undefined;
    const params: { workspace: string; provider: string; model?: string; title?: string } = { workspace: tab.workspace, provider };
    if (model) params.model = model;
    if (title) params.title = title;
    const r = await this.attempt('Cannot start the thread', () => this.connection.call('thread.create', params));
    if (!r) return undefined;
    this.connection.noteThread(r.thread);
    this.openTab(tab.workspace, r.thread.id);
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
      'core.mismatch': this.connection.status === 'mismatch',
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
    c.register({ id: 'theme.system', title: 'Theme: follow the system' }, () => this.selectTheme(undefined, 'system'));
    c.register({ id: 'core.restart', title: 'Restart the core', when: 'core.mismatch' }, async () => {
      const ok = await this.connection.restartMismatchedCore();
      if (ok) await this.connection.start();
      else this.notify('This core cannot be restarted from here. Stop the running `kumikit serve` and reopen the window.', 'warning');
    });
    c.register({ id: 'approval.respond', title: 'Answer an approval', when: 'never' }, async (args) => {
      const a = args as { approval: string; decision: 'allow' | 'allow_always' | 'deny' };
      await this.attempt('Cannot answer the approval', () => this.threads.respond(a.approval, a.decision));
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
