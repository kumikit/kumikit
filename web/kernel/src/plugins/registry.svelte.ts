/**
 * Plugins from registries in the plugin manager (06-client, Plugin manager; 10-distribution):
 * browse and search, install, check for updates, registries, lockfiles, for the chosen core
 * (through `registry.*` and `plugin.*`) and for this app (through the shell).
 *
 * Nothing here runs by itself: every method is a user action, and registries are
 * contacted only inside one (the README's privacy section).
 */
import type { AvailableUpdate, InstalledPlugin, RegistryInfo, RegistryResult } from '@kumikit/protocol';

import type { Kernel } from '../app.svelte';
import type { ClientRegistryResult, ClientUpdate } from '../platform/index';
import type { ClientPlugin } from './client.svelte';
import { isAgentResult } from './agents';
import { type Lock, type LockEntry, parseLock, registryOf, withClientFlags } from './lockfile';

/** What importing a lockfile will do with one entry. */
export interface ImportItem {
  entry: LockEntry;
  /** `install`, or why it is skipped. */
  action: 'install' | 'installed' | 'other-version' | 'not-from-registry' | 'unknown-registry';
}

/** Decides what an import does, before anything happens. Pure, for tests. */
export function importPlan(lock: Lock, installed: Map<string, string>, registries: string[]): ImportItem[] {
  return lock.plugins.map((entry): ImportItem => {
    const have = installed.get(entry.id);
    if (have === entry.version) return { entry, action: 'installed' };
    if (have !== undefined) return { entry, action: 'other-version' };
    const url = registryOf(entry);
    if (!url) return { entry, action: 'not-from-registry' };
    return { entry, action: registries.includes(url) ? 'install' : 'unknown-registry' };
  });
}

export class Registries {
  /** Results of the last search, or undefined before the user searched. */
  core = $state.raw<RegistryResult[] | undefined>();
  client = $state.raw<ClientRegistryResult[] | undefined>();
  searching = $state(false);
  /** Agents found by the last agent search (Settings → Agents), or undefined before it. */
  agents = $state.raw<RegistryResult[] | undefined>();
  searchingAgents = $state(false);
  /** Updates found by the last check, or undefined before the user checked. */
  coreUpdates = $state.raw<AvailableUpdate[] | undefined>();
  clientUpdates = $state.raw<ClientUpdate[] | undefined>();
  checking = $state(false);
  coreRegistries = $state.raw<RegistryInfo[] | undefined>();
  clientRegistries = $state.raw<string[] | undefined>();

  constructor(private readonly kernel: Kernel) {}

  /** Whether the chosen core reads registries (`registry` capability). */
  get coreAvailable(): boolean {
    return this.kernel.corePlugins.target.hello?.capabilities.includes('registry') ?? false;
  }

  /** Whether this app installs UI plugins from registries (not in a browser). */
  get clientAvailable(): boolean {
    return this.kernel.platform.registry !== undefined;
  }

  /** Searches the chosen core's registries and this app's. */
  async search(query: string, refresh = false): Promise<void> {
    const k = this.kernel;
    const c = k.corePlugins.target;
    this.searching = true;
    const [core, client] = await Promise.all([
      this.coreAvailable
        ? k.attempt('Cannot search the registries', () => c.call('registry.search', { query, refresh }))
        : Promise.resolve(undefined),
      this.clientAvailable
        ? k.attempt('Cannot search the registries', () => k.platform.registry!.search(query, refresh))
        : Promise.resolve(undefined),
    ]);
    this.searching = false;
    this.core = core?.results ?? (this.coreAvailable ? [] : undefined);
    this.client = client ?? (this.clientAvailable ? [] : undefined);
  }

  /** Searches the chosen core's registries for agents only (Settings → Agents). */
  async searchAgents(query: string, refresh = false): Promise<void> {
    const k = this.kernel;
    const c = k.corePlugins.target;
    if (!this.coreAvailable) return;
    this.searchingAgents = true;
    const r = await k.attempt('Cannot search the registries', () => c.call('registry.search', { query, refresh }));
    this.searchingAgents = false;
    this.agents = (r?.results ?? []).filter(isAgentResult);
  }

  /** Installs a core plugin on the chosen core, from the registry it was found in.
   * Nothing is granted: the caller shows the permissions. */
  async installCore(r: RegistryResult): Promise<InstalledPlugin | undefined> {
    const k = this.kernel;
    const c = k.corePlugins.target;
    const got = await k.attempt('Cannot install', () => c.call('plugin.install', { registry: { id: r.id, registry: r.registry } }));
    await k.corePlugins.refresh();
    return got?.plugin;
  }

  /** Installs a UI plugin or theme in this app, from the registry it was found in. */
  async installClient(r: ClientRegistryResult): Promise<ClientPlugin | undefined> {
    return this.kernel.clientPlugins.installFrom({ id: r.id, registry: r.registry });
  }

  /** Asks the registries for updates; applies nothing. */
  async checkUpdates(): Promise<void> {
    const k = this.kernel;
    const c = k.corePlugins.target;
    this.checking = true;
    const [core, client] = await Promise.all([
      this.coreAvailable ? k.attempt('Cannot check for updates', () => c.call('plugin.updates', { refresh: true })) : Promise.resolve(undefined),
      this.clientAvailable ? k.attempt('Cannot check for updates', () => k.platform.registry!.updates(true)) : Promise.resolve(undefined),
    ]);
    this.checking = false;
    this.coreUpdates = core?.updates ?? [];
    this.clientUpdates = client ?? [];
  }

  /** Applies a core plugin's update. One that widens permissions grants the new ones,
   * so call it only after the user agreed to them. */
  async updateCore(u: AvailableUpdate): Promise<boolean> {
    const k = this.kernel;
    const c = k.corePlugins.target;
    const params = u.widens ? { plugin: u.plugin, version: u.to, grant: u.requested } : { plugin: u.plugin, version: u.to };
    const r = await k.attempt(`Cannot update ${u.plugin}`, () => c.call('plugin.update', params));
    await k.corePlugins.refresh();
    if (!r) return false;
    this.coreUpdates = this.coreUpdates?.filter((x) => x.plugin !== u.plugin);
    k.notify(`Updated ${r.plugin.name} to ${r.plugin.version}`, 'success');
    return true;
  }

  /** Applies a UI plugin's update (after the user agreed, if it widens). */
  async updateClient(u: ClientUpdate): Promise<boolean> {
    const k = this.kernel;
    const entry = await k.attempt(`Cannot update ${u.plugin}`, () => k.platform.registry!.update(u.plugin, u.to));
    if (!entry) return false;
    await k.clientPlugins.updated(entry);
    this.clientUpdates = this.clientUpdates?.filter((x) => x.plugin !== u.plugin);
    k.notify(`Updated ${u.plugin} to ${u.to}`, 'success');
    return true;
  }

  async loadRegistries(): Promise<void> {
    const k = this.kernel;
    const c = k.corePlugins.target;
    if (this.coreAvailable) this.coreRegistries = (await k.attempt('Cannot list registries', () => c.call('registry.list', {})))?.registries;
    if (this.clientAvailable) this.clientRegistries = await k.attempt('Cannot list registries', () => k.platform.registry!.registries());
  }

  async addCoreRegistry(url: string): Promise<void> {
    const c = this.kernel.corePlugins.target;
    await this.kernel.attempt('Cannot add the registry', () => c.call('registry.add', { url }));
    await this.loadRegistries();
  }

  async removeCoreRegistry(url: string): Promise<void> {
    const c = this.kernel.corePlugins.target;
    await this.kernel.attempt('Cannot remove the registry', () => c.call('registry.remove', { url }));
    await this.loadRegistries();
  }

  async setClientRegistries(urls: string[]): Promise<void> {
    await this.kernel.attempt('Cannot change the registries', () => this.kernel.platform.registry!.setRegistries(urls));
    await this.loadRegistries();
  }

  /** The chosen core's lockfile. */
  async coreLock(): Promise<string | undefined> {
    const c = this.kernel.corePlugins.target;
    return (await this.kernel.attempt('Cannot read the lockfile', () => c.call('plugin.lock', {})))?.lock;
  }

  /** This app's lockfile, with each plugin's enabled and trusted flags. */
  async clientLock(): Promise<string | undefined> {
    const k = this.kernel;
    const text = await k.attempt('Cannot read the lockfile', () => k.platform.registry!.lock());
    if (text === undefined) return undefined;
    const s = k.settings.value;
    return withClientFlags(text, (id) => ({ enabled: !s.disabledPlugins.includes(id), trusted: s.trustedPlugins.includes(id) }));
  }

  /** What importing `text` into the chosen core would do. */
  async corePlan(text: string): Promise<ImportItem[]> {
    const lock = parseLock(text);
    const installed = new Map(this.kernel.corePlugins.plugins.map((p) => [p.id, p.version]));
    await this.loadRegistries();
    return importPlan(lock, installed, (this.coreRegistries ?? []).map((r) => r.url));
  }

  /** What importing `text` into this app would do. */
  async clientPlan(text: string): Promise<ImportItem[]> {
    const lock = parseLock(text);
    const installed = new Map(this.kernel.clientPlugins.plugins.filter((p) => p.entry.source === 'installed').map((p) => [p.entry.id, p.entry.version]));
    await this.loadRegistries();
    return importPlan(lock, installed, this.clientRegistries ?? []);
  }

  /** Installs a plan's entries on the chosen core at their locked versions and hashes.
   * Returns the installed plugins that were enabled on the other machine: the caller
   * shows their permissions together and enables them once the user agrees. */
  async importCore(plan: ImportItem[]): Promise<InstalledPlugin[]> {
    const k = this.kernel;
    const c = k.corePlugins.target;
    const toEnable: InstalledPlugin[] = [];
    for (const { entry } of plan.filter((i) => i.action === 'install')) {
      const registry = { id: entry.id, version: entry.version, sha256: entry.sha256, registry: registryOf(entry) };
      const r = await k.attempt(`Cannot install ${entry.id}`, () => c.call('plugin.install', { registry }));
      if (r && entry.state === 'enabled') toEnable.push(r.plugin);
    }
    await k.corePlugins.refresh();
    return toEnable;
  }

  /** Installs a plan's entries in this app. Inline plugins that were trusted are not
   * trusted here until the user says so; disabled ones stay disabled. */
  async importClient(plan: ImportItem[]): Promise<number> {
    const k = this.kernel;
    let n = 0;
    for (const { entry } of plan.filter((i) => i.action === 'install')) {
      const p = await k.clientPlugins.installFrom({ id: entry.id, version: entry.version, sha256: entry.sha256, registry: registryOf(entry) });
      if (!p) continue;
      n++;
      if (entry.enabled === false) await k.clientPlugins.setEnabled(p.entry.id, false);
    }
    return n;
  }
}
