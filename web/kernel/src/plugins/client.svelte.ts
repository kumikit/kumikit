/**
 * UI plugins on this client (06-client, Plugin manager): which are enabled, activating
 * inline modules with a plugin context, and mounting sandboxed iframe panels.
 * Themes are plain resource files loaded separately by the theme engine (ADR 0037).
 */
import type { Plugin, SettingsSchema } from '@kumikit/ui-sdk';
import { mount, unmount } from 'svelte';

import type { Kernel } from '../app.svelte';
import Form from '../declarative/Form.svelte';
import type { ClientPluginEntry, ClientRef } from '../platform/index';
import { createContext } from './context';
import { PANEL_SLOTS, mountPanel } from './panel';

/** What the UI kernel reads from a UI plugin manifest. */
export interface ClientManifest {
  id: string;
  name: string;
  version: string;
  description?: string;
  repository?: string;
  kind: ['ui'];
  ui?: { entry: string; isolation?: string; network?: string[] };
  contributes?: { settings?: string; slots?: { slot?: unknown; id?: unknown; title?: unknown }[] };
  /** Core plugins this UI plugin needs, `<id>@<range>` (07-ui-plugins). */
  requires?: { core?: string[] };
}

export interface ClientPlugin {
  entry: ClientPluginEntry;
  manifest: ClientManifest;
  kind: 'ui';
  enabled: boolean;
  /** Third-party inline code waiting for the user's trust. */
  needsTrust: boolean;
  /** Why it is not running, if it failed. */
  error?: string;
}

/** Data shipped with the app next to the UI kernel. */
export interface Bundled {
  fallbackLight?: string;
  fallbackDark?: string;
}

export class ClientPlugins {
  plugins = $state.raw<ClientPlugin[]>([]);
  bundled: Bundled = {};
  #active = new Map<string, { plugin: Plugin; dispose: (() => void)[] }>();

  constructor(private readonly kernel: Kernel) {}

  /** A plugin's `requires.core` entries, if it has any. */
  requiresOf(id: string): string[] | undefined {
    const core = this.plugins.find((p) => p.entry.id === id)?.manifest.requires?.core;
    return Array.isArray(core) ? core.filter((x): x is string => typeof x === 'string') : undefined;
  }

  /** Reads `bundled.json`, loads bundled + custom themes, and activates enabled UI plugins. */
  async load(): Promise<void> {
    const k = this.kernel;
    try {
      const r = await fetch('./bundled.json');
      if (r.ok) this.bundled = (await r.json()) as Bundled;
    } catch {
      // No bundled.json (development): default fallback ids apply.
    }
    if (this.bundled.fallbackLight) k.theme.fallbackLight = this.bundled.fallbackLight;
    if (this.bundled.fallbackDark) k.theme.fallbackDark = this.bundled.fallbackDark;
    await k.loadThemes();
    let entries: ClientPluginEntry[] = [];
    try {
      entries = await k.platform.clientPlugins.list();
    } catch (e) {
      k.notify(`Cannot list UI plugins: ${String(e)}`, 'danger');
    }
    this.plugins = entries
      .filter((entry) => {
        const m = entry.manifest as Partial<ClientManifest>;
        return Array.isArray(m.kind) && m.kind[0] === 'ui';
      })
      .map((entry) => this.#describe(entry));
    if (k.platform.safeMode) return;
    for (const p of this.plugins) if (p.enabled) await this.#activate(p);
  }

  #describe(entry: ClientPluginEntry): ClientPlugin {
    const manifest = entry.manifest as ClientManifest;
    const s = this.kernel.settings.value;
    // Only inline code asks for trust; a sandboxed panel cannot reach the app (ADR 0019).
    const inline = (manifest.ui?.isolation ?? 'inline') === 'inline';
    const needsTrust = inline && entry.source === 'installed' && !s.trustedPlugins.includes(entry.id);
    return { entry, manifest, kind: 'ui', enabled: !s.disabledPlugins.includes(entry.id) && !needsTrust, needsTrust };
  }

  #update(id: string, change: Partial<ClientPlugin>): void {
    this.plugins = this.plugins.map((p) => (p.entry.id === id ? { ...p, ...change } : p));
  }

  async #activate(p: ClientPlugin): Promise<void> {
    const k = this.kernel;
    const id = p.entry.id;
    const dispose: (() => void)[] = [];
    try {
      const entry = p.manifest.ui?.entry;
      if (!entry) throw new Error('the manifest names no entry');
      if (p.manifest.ui?.isolation === 'iframe') {
        this.#activatePanels(p, entry, dispose);
        this.#active.set(id, { plugin: { activate() {} }, dispose });
        this.#update(id, { error: undefined });
        return;
      }
      const module = (await import(/* @vite-ignore */ k.platform.pluginUrl(id, entry))) as { default?: Plugin };
      const plugin = module.default;
      if (!plugin || typeof plugin.activate !== 'function') throw new Error('its module exports no plugin');
      const ctx = createContext(k, id, dispose);
      this.#active.set(id, { plugin, dispose });
      await plugin.activate(ctx);
      const schema = p.manifest.contributes?.settings;
      if (schema) dispose.push(await this.#settingsPage(p, schema));
      this.#update(id, { error: undefined });
    } catch (e) {
      const message = e instanceof Error ? e.message : String(e);
      this.#update(id, { error: message });
      if (k.slots.fail(id, e)) return;
      k.notify(`The UI plugin ${p.manifest.name} could not start: ${message}`, 'danger');
    }
  }

  /** A sandboxed plugin's slots, from its manifest: each mounts a panel when shown. Its
   * code loads only then, inside the frame. */
  #activatePanels(p: ClientPlugin, entry: string, dispose: (() => void)[]): void {
    const k = this.kernel;
    const id = p.entry.id;
    const plugin = { id, entry, requires: (this.requiresOf(id) ?? []).map((r) => r.split('@')[0]!) };
    for (const c of p.manifest.contributes?.slots ?? []) {
      const slot = String(c.slot);
      if (!(PANEL_SLOTS as readonly string[]).includes(slot) || typeof c.id !== 'string') {
        k.notify(`${p.manifest.name}: a sandboxed panel cannot go in ${slot}`, 'warning');
        continue;
      }
      const title = typeof c.title === 'string' ? c.title : c.id;
      const contribution = { id: c.id, title, mount: (el: HTMLElement) => mountPanel(k, plugin, slot, { id: c.id as string, title }, el) };
      dispose.push(k.slots.register(id, slot as 'inspector.tab', contribution));
    }
  }

  /** A settings page drawn from the plugin's declarative schema; values live in its
   * client storage. */
  async #settingsPage(p: ClientPlugin, path: string): Promise<() => void> {
    const k = this.kernel;
    const id = p.entry.id;
    const schema = (await (await fetch(k.platform.pluginUrl(id, path))).json()) as SettingsSchema;
    return k.slots.register(id, 'settings.page', {
      id: `${id}.settings`,
      title: p.manifest.name,
      mount(el) {
        const stored = k.settings.value.pluginStorage[id]?.['settings'];
        const values = stored ? (JSON.parse(stored) as Record<string, unknown>) : {};
        const c = mount(Form, {
          target: el,
          props: {
            schema,
            values,
            onchange: (v: Record<string, unknown>) => {
              const all = k.settings.value.pluginStorage;
              k.settings.update({ pluginStorage: { ...all, [id]: { ...all[id], settings: JSON.stringify(v) } } });
            },
          },
        });
        return () => void unmount(c);
      },
    });
  }

  /** Stops a plugin's contributions now; its module stays loaded until a restart. */
  async #deactivate(id: string): Promise<void> {
    const a = this.#active.get(id);
    this.#active.delete(id);
    try {
      await a?.plugin.deactivate?.();
    } catch {
      // A failing deactivate does not keep its contributions alive.
    }
    for (const d of a?.dispose ?? []) d();
    this.kernel.slots.removePlugin(id);
    this.kernel.commands.removeSource(id);
  }

  async setEnabled(id: string, enabled: boolean): Promise<void> {
    const k = this.kernel;
    const s = k.settings.value;
    const disabled = s.disabledPlugins.filter((x) => x !== id);
    if (!enabled) disabled.push(id);
    k.settings.update({ disabledPlugins: disabled });
    const p = this.plugins.find((x) => x.entry.id === id);
    if (!p) return;
    this.#update(id, { enabled });
    if (enabled && !k.platform.safeMode) await this.#activate({ ...p, enabled });
    else await this.#deactivate(id);
  }

  /** Trusts an installed inline plugin and enables it. */
  async trust(id: string): Promise<void> {
    const s = this.kernel.settings.value;
    this.kernel.settings.update({ trustedPlugins: [...new Set([...s.trustedPlugins, id])] });
    this.#update(id, { needsTrust: false });
    await this.setEnabled(id, true);
  }

  /** Called when the slot registry gives up on a plugin. */
  async failed(id: string): Promise<void> {
    await this.#deactivate(id);
    const s = this.kernel.settings.value;
    this.kernel.settings.update({ disabledPlugins: [...new Set([...s.disabledPlugins, id])] });
    this.#update(id, { enabled: false });
  }

  /** Installs a package picked by the user; third-party inline code waits for trust. */
  async install(path: string): Promise<ClientPlugin | undefined> {
    const k = this.kernel;
    const entry = await k.attempt('Cannot install', () => k.platform.clientPlugins.install(path));
    return entry ? this.added(entry) : undefined;
  }

  /** Installs a UI plugin from the app's registries. */
  async installFrom(plugin: ClientRef): Promise<ClientPlugin | undefined> {
    const k = this.kernel;
    const registry = k.platform.registry;
    if (!registry) return undefined;
    const entry = await k.attempt('Cannot install', () => registry.install(plugin));
    return entry ? this.added(entry) : undefined;
  }

  /** After an update: the new version replaces the old in the list. An update that brings
   * inline code no longer counts as trusted; the user is asked again (10-distribution,
   * Updates). Its old module stays loaded until the app restarts. */
  async updated(entry: ClientPluginEntry): Promise<ClientPlugin> {
    const k = this.kernel;
    const p = this.#describe(entry);
    if ((p.manifest.ui?.isolation ?? 'inline') === 'inline') {
      const was = this.plugins.find((x) => x.entry.id === entry.id);
      const wasInline = was && (was.manifest.ui?.isolation ?? 'inline') === 'inline';
      if (!wasInline) {
        k.settings.update({ trustedPlugins: k.settings.value.trustedPlugins.filter((x) => x !== entry.id) });
        await this.#deactivate(entry.id);
      }
    }
    const now = this.#describe(entry);
    this.plugins = this.plugins.map((x) => (x.entry.id === entry.id ? now : x));
    return now;
  }

  /** Adds an installed UI plugin to the list and activates it if trusted or sandboxed. */
  async added(entry: ClientPluginEntry): Promise<ClientPlugin> {
    const k = this.kernel;
    const p = this.#describe(entry);
    this.plugins = [...this.plugins.filter((x) => x.entry.id !== entry.id), p];
    // A sandboxed panel needs no trust, so it starts at once (inline code waits for trust).
    if (p.enabled && !k.platform.safeMode) await this.#activate(p);
    k.notify(`Installed ${p.manifest.name}`, 'success');
    return p;
  }

  async remove(id: string): Promise<void> {
    const k = this.kernel;
    await this.#deactivate(id);
    const ok = await k.attempt('Cannot remove', () => k.platform.clientPlugins.remove(id).then(() => true));
    if (!ok) return;
    this.plugins = this.plugins.filter((p) => p.entry.id !== id);
  }
}
