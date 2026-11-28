/** Core plugins on a chosen core (06-client, Plugin manager; 02-protocol `plugin.*`).
 * Permissions are granted as a whole: the grant is exactly what the plugin requests. */
import type { InstalledPlugin } from '@kumikit/protocol';

import type { Kernel } from '../app.svelte';
import type { CoreConnection } from '../connections/connection.svelte';
import { toBase64 } from '../platform/index';

export class CorePlugins {
  plugins = $state.raw<InstalledPlugin[]>([]);
  loading = $state(false);
  /** The core the plugin manager shows; the active tab's until the user picks another. */
  chosen = $state<string | undefined>();

  constructor(private readonly kernel: Kernel) {}

  /** The connection being managed. */
  get target(): CoreConnection {
    return this.kernel.connectionFor(this.chosen) ?? this.kernel.connection;
  }

  /** Whether packages are installed from a path on this machine (the local core) or
   * uploaded (a core on another machine, or the core serving a browser). */
  get uploads(): boolean {
    return this.target.kind !== 'local';
  }

  choose(id: string): void {
    this.chosen = id;
    this.plugins = [];
    // Agent search results belong to the core they came from.
    this.kernel.registries.agents = undefined;
    void this.refresh();
  }

  async refresh(): Promise<void> {
    const c = this.target;
    this.loading = true;
    const r = await this.kernel.attempt('Cannot list core plugins', () => c.call('plugin.list', {}));
    this.loading = false;
    if (r && c === this.target) this.plugins = r.plugins;
  }

  /** Installs a package or plugin folder from a path on this machine; nothing is granted yet. */
  async install(path: string): Promise<InstalledPlugin | undefined> {
    const c = this.target;
    const r = await this.kernel.attempt('Cannot install', () => c.call('plugin.install', { path }));
    await this.refresh();
    return r?.plugin;
  }

  /** Installs a `.kkp` the user picked by sending its bytes to the core. */
  async upload(bytes: Uint8Array): Promise<InstalledPlugin | undefined> {
    const c = this.target;
    const r = await this.kernel.attempt('Cannot install', () => c.call('plugin.install', { package: toBase64(bytes) }));
    await this.refresh();
    return r?.plugin;
  }

  async enable(p: InstalledPlugin): Promise<boolean> {
    const c = this.target;
    const r = await this.kernel.attempt('Cannot enable', () => c.call('plugin.enable', { plugin: p.id, grant: p.requested }));
    await this.refresh();
    if (r) this.kernel.notify(`${p.name} is enabled on ${c.label}`, 'success');
    return r !== undefined;
  }

  async disable(id: string): Promise<void> {
    const c = this.target;
    await this.kernel.attempt('Cannot disable', () => c.call('plugin.disable', { plugin: id }));
    await this.refresh();
  }

  async uninstall(id: string, deleteData: boolean): Promise<void> {
    const c = this.target;
    await this.kernel.attempt('Cannot remove', () => c.call('plugin.uninstall', { plugin: id, deleteData }));
    await this.refresh();
  }
}
