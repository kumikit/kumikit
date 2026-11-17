/**
 * UI plugins and themes on this client (06-client, Plugin manager): which are enabled,
 * loading themes into the theme engine and activating inline modules with a plugin
 * context. Nothing here names a plugin; `bundled.json` names the fallback theme.
 */
import type { Plugin, SettingsSchema } from '@kumikit/ui-sdk';
import { mount, unmount } from 'svelte';

import type { Kernel } from '../app.svelte';
import Form from '../declarative/Form.svelte';
import type { ClientPluginEntry } from '../platform/index';
import type { ThemeVariant } from '../theme/engine.svelte';
import { checkTokens } from '../theme/validate';
import { createContext } from './context';

/** What the UI kernel reads from a UI plugin or theme manifest. */
export interface ClientManifest {
  id: string;
  name: string;
  version: string;
  description?: string;
  repository?: string;
  kind: ['ui'] | ['theme'];
  ui?: { entry: string; isolation?: string };
  theme?: { variants: { id: string; name?: string; appearance: 'light' | 'dark'; tokens: string }[] };
  contributes?: { settings?: string };
}

export interface ClientPlugin {
  entry: ClientPluginEntry;
  manifest: ClientManifest;
  kind: 'ui' | 'theme';
  enabled: boolean;
  /** Third-party inline code waiting for the user's trust. */
  needsTrust: boolean;
  /** Why it is not running, if it failed. */
  error?: string;
}

/** Data shipped with the app next to the UI kernel. */
export interface Bundled {
  fallbackTheme?: string;
}

export class ClientPlugins {
  plugins = $state.raw<ClientPlugin[]>([]);
  bundled: Bundled = {};
  #active = new Map<string, { plugin: Plugin; dispose: (() => void)[] }>();

  constructor(private readonly kernel: Kernel) {}

  /** Reads the list, loads the themes and activates the enabled UI plugins. */
  async load(): Promise<void> {
    const k = this.kernel;
    try {
      const r = await fetch('./bundled.json');
      if (r.ok) this.bundled = (await r.json()) as Bundled;
    } catch {
      // No bundled.json (development): no fallback theme; system colors apply.
    }
    k.theme.fallbackTheme = this.bundled.fallbackTheme;
    let entries: ClientPluginEntry[] = [];
    try {
      entries = await k.platform.clientPlugins.list();
    } catch (e) {
      k.notify(`Cannot list UI plugins: ${String(e)}`, 'danger');
    }
    this.plugins = entries.map((entry) => this.#describe(entry));
    await this.#loadThemes();
    if (k.platform.safeMode) return;
    for (const p of this.plugins) if (p.kind === 'ui' && p.enabled) await this.#activate(p);
  }

  #describe(entry: ClientPluginEntry): ClientPlugin {
    const manifest = entry.manifest as ClientManifest;
    const kind = manifest.kind[0];
    const s = this.kernel.settings.value;
    const needsTrust = kind === 'ui' && entry.source === 'installed' && !s.trustedPlugins.includes(entry.id);
    return { entry, manifest, kind, enabled: !s.disabledPlugins.includes(entry.id) && !needsTrust, needsTrust };
  }

  #update(id: string, change: Partial<ClientPlugin>): void {
    this.plugins = this.plugins.map((p) => (p.entry.id === id ? { ...p, ...change } : p));
  }

  async #loadThemes(): Promise<void> {
    const k = this.kernel;
    const variants: ThemeVariant[] = [];
    for (const p of this.plugins.filter((p) => p.kind === 'theme' && p.enabled)) {
      for (const v of p.manifest.theme?.variants ?? []) {
        try {
          const r = await fetch(k.platform.pluginUrl(p.entry.id, v.tokens));
          const checked = checkTokens(await r.json());
          if (checked.errors.length > 0) {
            k.notify(`Theme ${p.manifest.name}: ${checked.errors.length} token(s) ignored (${checked.errors.slice(0, 3).join('; ')})`, 'warning');
          }
          variants.push({
            theme: p.entry.id,
            id: v.id,
            name: `${p.manifest.name} ${v.name ?? v.id.charAt(0).toUpperCase() + v.id.slice(1)}`,
            appearance: v.appearance,
            tokens: checked.tokens,
          });
        } catch (e) {
          this.#update(p.entry.id, { error: `cannot read ${v.tokens}: ${String(e)}` });
        }
      }
    }
    k.theme.setVariants(variants);
    k.refreshThemeCommands();
    k.applyTheme();
  }

  async #activate(p: ClientPlugin): Promise<void> {
    const k = this.kernel;
    const id = p.entry.id;
    const dispose: (() => void)[] = [];
    try {
      const entry = p.manifest.ui?.entry;
      if (!entry) throw new Error('the manifest names no entry');
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
    if (p.kind === 'theme') await this.#loadThemes();
    else if (enabled && !k.platform.safeMode) await this.#activate({ ...p, enabled });
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

  /** Installs a package picked by the user. Themes are enabled at once; third-party
   * inline code waits for trust. */
  async install(path: string): Promise<ClientPlugin | undefined> {
    const k = this.kernel;
    const entry = await k.attempt('Cannot install', () => k.platform.clientPlugins.install(path));
    if (!entry) return undefined;
    const p = this.#describe(entry);
    this.plugins = [...this.plugins.filter((x) => x.entry.id !== entry.id), p];
    if (p.kind === 'theme') await this.#loadThemes();
    k.notify(`Installed ${p.manifest.name}`, 'success');
    return p;
  }

  async remove(id: string): Promise<void> {
    const k = this.kernel;
    await this.#deactivate(id);
    const ok = await k.attempt('Cannot remove', () => k.platform.clientPlugins.remove(id).then(() => true));
    if (!ok) return;
    const was = this.plugins.find((p) => p.entry.id === id);
    this.plugins = this.plugins.filter((p) => p.entry.id !== id);
    if (was?.kind === 'theme') await this.#loadThemes();
  }
}
