/**
 * Core plugins' commands in the palette (07-ui-plugins, Level 1; 02-protocol `command.*`).
 * The active tab's core lists them; running one shows its message and opens its view.
 * Nothing here names a plugin.
 */
import type { CommandInfo, CommandResult, WorkspaceId } from '@kumikit/protocol';
import type { DeclarativeView } from '@kumikit/ui-sdk';
import { untrack } from 'svelte';

import type { Kernel } from '../app.svelte';
import type { CoreConnection } from '../connections/connection.svelte';

/** The palette id of a core command. */
export const coreCommandId = (c: Pick<CommandInfo, 'plugin' | 'id'>) => `core:${c.plugin}:${c.id}`;

/** The `when` clause a core command gets: its own, and an open tab for `workspace` ones. */
export function coreWhen(c: CommandInfo): string {
  return [c.scope === 'workspace' ? 'tab.open' : '', c.when ?? ''].filter(Boolean).join(' && ');
}

/** A core command's view, opened in the main area. */
export interface CoreView {
  connection: string;
  plugin: string;
  workspace?: WorkspaceId;
  title: string;
  view: DeclarativeView;
}

export class CoreCommands {
  /** The commands registered now (for the active tab's core). */
  listed = $state.raw<CommandInfo[]>([]);
  #loaded: { connection: CoreConnection; plugins: unknown } | undefined;

  constructor(private readonly kernel: Kernel) {}

  /** Keeps the palette in step with the active core's plugins; call once. */
  watch(): () => void {
    return $effect.root(() => {
      $effect(() => {
        const c = this.kernel.connection;
        const plugins = c.plugins;
        const ready = c.ready && Boolean(c.hello?.capabilities.includes('commands'));
        untrack(() => void this.#sync(c, ready ? plugins : undefined));
      });
    });
  }

  async #sync(c: CoreConnection, plugins: unknown): Promise<void> {
    if (this.#loaded?.connection === c && this.#loaded.plugins === plugins) return;
    this.#loaded = { connection: c, plugins };
    let list: CommandInfo[] = [];
    if (plugins !== undefined) {
      try {
        list = (await c.call('command.list', {})).commands;
      } catch {
        list = [];
      }
    }
    if (this.#loaded?.connection !== c || this.#loaded.plugins !== plugins) return;
    const k = this.kernel;
    k.commands.removeSource('core');
    this.listed = list;
    for (const info of list) {
      k.commands.register(
        { id: coreCommandId(info), title: info.title, when: coreWhen(info) || undefined },
        (args) => this.run(c, info.plugin, info.id, args, info.scope === 'workspace' ? k.activeTab?.workspace : undefined, info.title),
        'core',
      );
    }
  }

  /** Runs a command on `c` and shows what it returns. */
  async run(c: CoreConnection, plugin: string, command: string, args: unknown, workspace: WorkspaceId | undefined, title: string): Promise<CommandResult | undefined> {
    const k = this.kernel;
    const params: { plugin: string; command: string; args?: Record<string, unknown>; workspace?: WorkspaceId } = { plugin, command };
    if (args && typeof args === 'object' && !Array.isArray(args)) params.args = args as Record<string, unknown>;
    if (workspace) params.workspace = workspace;
    const r = await k.attempt(`Cannot run ${title}`, () => c.call('command.run', params));
    if (!r) return undefined;
    if (r.message) k.notify(r.message, 'info');
    if (r.view) {
      const view: CoreView = { connection: c.id, plugin, title, view: r.view as unknown as DeclarativeView };
      if (workspace) view.workspace = workspace;
      k.main = { kind: 'core-view', view };
    }
    return r;
  }
}
