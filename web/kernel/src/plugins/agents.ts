/**
 * Which plugins are agents (06-client, Agents): a core plugin whose manifest declares a
 * provider, and a registry result of kind `provider`. Agents are managed in Settings →
 * Agents and left out of the plugin manager. Pure, for tests.
 */
import type { AvailableUpdate, InstalledPlugin, PluginInfo, RegistryResult } from '@kumikit/protocol';

/** Whether an installed core plugin is an agent: it declares a provider, enabled or not. */
export function isAgent(p: InstalledPlugin): boolean {
  return (p.providers?.length ?? 0) > 0 || p.contributes.providers.length > 0;
}

/** Whether an enabled plugin from `core.hello` is an agent (it serves a provider now). */
export function isAgentInfo(p: PluginInfo): boolean {
  return p.contributes.providers.length > 0;
}

/** Whether a registry result is an agent. */
export function isAgentResult(r: RegistryResult): boolean {
  return r.kind.includes('provider');
}

/** The updates that belong to installed agents (`agents: true`) or to other plugins. */
export function updatesFor(updates: AvailableUpdate[], installed: InstalledPlugin[], agents: boolean): AvailableUpdate[] {
  const ids = new Set(installed.filter(isAgent).map((p) => p.id));
  return updates.filter((u) => ids.has(u.plugin) === agents);
}
