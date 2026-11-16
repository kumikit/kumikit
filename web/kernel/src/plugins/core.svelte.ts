/** Core plugins on the active connection (06-client, Plugin manager; 02-protocol
 * `plugin.*`). Permissions are granted as a whole: the grant is exactly what the plugin
 * requests. */
import type { InstalledPlugin } from '@kumikit/protocol';

import type { Kernel } from '../app.svelte';

export class CorePlugins {
  plugins = $state.raw<InstalledPlugin[]>([]);
  loading = $state(false);

  constructor(private readonly kernel: Kernel) {}

  async refresh(): Promise<void> {
    this.loading = true;
    const r = await this.kernel.attempt('Cannot list core plugins', () => this.kernel.connection.call('plugin.list', {}));
    this.loading = false;
    if (r) this.plugins = r.plugins;
  }

  /** Installs a package or plugin folder on the core; nothing is granted yet. */
  async install(path: string): Promise<InstalledPlugin | undefined> {
    const r = await this.kernel.attempt('Cannot install', () => this.kernel.connection.call('plugin.install', { path }));
    await this.refresh();
    return r?.plugin;
  }

  async enable(p: InstalledPlugin): Promise<boolean> {
    const r = await this.kernel.attempt('Cannot enable', () =>
      this.kernel.connection.call('plugin.enable', { plugin: p.id, grant: p.requested }),
    );
    await this.refresh();
    if (r) this.kernel.notify(`${p.name} is enabled`, 'success');
    return r !== undefined;
  }

  async disable(id: string): Promise<void> {
    await this.kernel.attempt('Cannot disable', () => this.kernel.connection.call('plugin.disable', { plugin: id }));
    await this.refresh();
  }

  async uninstall(id: string, deleteData: boolean): Promise<void> {
    await this.kernel.attempt('Cannot remove', () => this.kernel.connection.call('plugin.uninstall', { plugin: id, deleteData }));
    await this.refresh();
  }
}
