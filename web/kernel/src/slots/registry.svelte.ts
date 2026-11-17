/**
 * Where plugins contribute (07-ui-plugins, Slots). A contribution that throws is shown
 * as an error placeholder; after 3 failures in a session its plugin is disabled.
 */
import type { Slot, SlotContributions } from '@kumikit/ui-sdk';

/** Failures in one session after which a plugin is disabled. */
export const MAX_FAILURES = 3;

export interface Entry<S extends Slot = Slot> {
  plugin: string;
  slot: S;
  contribution: SlotContributions[S];
}

export class SlotRegistry {
  entries = $state.raw<Entry[]>([]);
  failures = $state.raw<Record<string, number>>({});

  constructor(private readonly onDisable: (plugin: string, reason: string) => void = () => {}) {}

  register<S extends Slot>(plugin: string, slot: S, contribution: SlotContributions[S]): () => void {
    const entry = { plugin, slot, contribution } as Entry;
    this.entries = [...this.entries, entry];
    return () => {
      this.entries = this.entries.filter((e) => e !== entry);
    };
  }

  /** Contributions to a slot, in registration order. */
  in<S extends Slot>(slot: S): Entry<S>[] {
    return this.entries.filter((e) => e.slot === slot) as Entry<S>[];
  }

  /** The `transcript.renderer` for an item kind, among the plugins `allow` lets through:
   * an exact match first, then the `plugin.event` fallback for plugin kinds. */
  renderer(kind: string, allow: (plugin: string) => boolean = () => true): Entry<'transcript.renderer'> | undefined {
    const all = this.in('transcript.renderer').filter((e) => allow(e.plugin));
    return (
      all.find((e) => e.contribution.kind === kind) ??
      (kind.startsWith('plugin.event:') ? all.find((e) => e.contribution.kind === 'plugin.event') : undefined)
    );
  }

  removePlugin(plugin: string): void {
    this.entries = this.entries.filter((e) => e.plugin !== plugin);
  }

  /** Records a failure; returns whether the plugin was disabled because of it. */
  fail(plugin: string, error: unknown): boolean {
    const n = (this.failures[plugin] ?? 0) + 1;
    this.failures = { ...this.failures, [plugin]: n };
    console.warn(`kumikit: plugin ${plugin} failed`, error);
    if (n >= MAX_FAILURES) {
      this.removePlugin(plugin);
      this.onDisable(plugin, String(error instanceof Error ? error.message : error));
      return true;
    }
    return false;
  }
}
