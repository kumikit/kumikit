/** Pure helpers for the model picker (06-client, Model picker; ADR 0033). */
import type { ModelInfo, ModelOption, ProviderInfo } from '@kumikit/protocol';

/** A recently used provider and model on one connection. */
export interface RecentModel {
  provider: string;
  model: string;
}

/** How many recent models the picker keeps per connection. */
export const RECENT_LIMIT = 8;

/** Puts `entry` first, without duplicates, keeping at most `limit`. */
export function pushRecent(list: readonly RecentModel[], entry: RecentModel, limit = RECENT_LIMIT): RecentModel[] {
  const rest = list.filter((r) => r.provider !== entry.provider || r.model !== entry.model);
  return [entry, ...rest].slice(0, limit);
}

/** Effort first, then speed, then any other settings in the provider's order. */
export function orderOptions(options: readonly ModelOption[] | undefined): ModelOption[] {
  const rank = (o: ModelOption) => (o.kind === 'effort' ? 0 : o.kind === 'speed' ? 1 : 2);
  return [...(options ?? [])].map((o, i) => [o, i] as const).sort((a, b) => rank(a[0]) - rank(b[0]) || a[1] - b[1]).map(([o]) => o);
}

/** The settings of `next` that are still valid for `model`, after a model change. */
export function keepValidOptions(model: ModelInfo | undefined, next: Readonly<Record<string, string>>): Record<string, string> {
  const out: Record<string, string> = {};
  for (const [id, value] of Object.entries(next)) {
    const option = model?.options?.find((o) => o.id === id);
    if (option?.values.some((v) => v.value === value)) out[id] = value;
  }
  return out;
}

/** One segment of a setting's row. `value` undefined is the model's default. */
export interface OptionChoice {
  value: string | undefined;
  label: string;
  description?: string;
}

/**
 * The choices shown for a setting: its values, led by "Default" (unset) unless the
 * provider already offers a value named `default`, which then stands for unset too.
 */
export function optionChoices(option: ModelOption): OptionChoice[] {
  const hasDefault = option.values.some((v) => v.value === 'default');
  const choices: OptionChoice[] = hasDefault ? [] : [{ value: undefined, label: 'Default' }];
  for (const v of option.values) {
    const c: OptionChoice = { value: v.value === 'default' ? undefined : v.value, label: v.label };
    if (v.description) c.description = v.description;
    choices.push(c);
  }
  return choices;
}

/** A model in search results or the recent list. */
export interface ModelEntry {
  provider: ProviderInfo;
  model: ModelInfo;
}

/** Models whose label, id or provider name contains every word of `query`. */
export function searchModels(entries: readonly ModelEntry[], query: string): ModelEntry[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return [...entries];
  return entries.filter((e) => {
    const hay = `${e.model.label} ${e.model.id} ${e.provider.name}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

/** The letter shown for a provider without an icon. */
export function providerInitial(name: string): string {
  return (name.trim()[0] ?? '?').toUpperCase();
}

/** The label of `model` among `models`, or its id. */
export function modelLabel(models: readonly ModelInfo[] | undefined, model: string): string {
  return models?.find((m) => m.id === model)?.label ?? model;
}
