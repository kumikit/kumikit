/**
 * `requires.core` (07-ui-plugins, Manifest): entries `<plugin id>@<semver range>`, met on
 * a connection when that core reports the plugin enabled at a version in the range.
 * Ranges: `1.2.3` (exactly), `^1.2`, `~1.2.3`, `>=1.2`, `*`, or none (`kumikit.git`).
 */
import type { PluginInfo } from '@kumikit/protocol';

type Version = [number, number, number];

function parse(v: string): { version: Version; parts: number } | undefined {
  const core = v.trim().split(/[-+]/)[0] ?? '';
  const bits = core.split('.');
  if (bits.length < 1 || bits.length > 3 || bits.some((b) => !/^\d+$/.test(b))) return undefined;
  const n = bits.map(Number);
  return { version: [n[0] ?? 0, n[1] ?? 0, n[2] ?? 0], parts: bits.length };
}

function cmp(a: Version, b: Version): number {
  for (let i = 0; i < 3; i++) if (a[i]! !== b[i]!) return a[i]! < b[i]! ? -1 : 1;
  return 0;
}

/** Whether `version` is in `range`. An unreadable range or version never matches. */
export function satisfies(version: string, range: string): boolean {
  const v = parse(version)?.version;
  if (!v) return false;
  const r = range.trim();
  if (r === '' || r === '*') return true;
  const op = /^(\^|~|>=|=)?(.*)$/.exec(r);
  const want = op ? parse(op[2] ?? '') : undefined;
  if (!op || !want) return false;
  const w = want.version;
  switch (op[1]) {
    case '>=':
      return cmp(v, w) >= 0;
    case '^': {
      // The first non-zero part is fixed: ^1.2 → <2, ^0.3 → <0.4, ^0.0.3 → exactly 0.0.3.
      if (cmp(v, w) < 0) return false;
      if (w[0] > 0 || want.parts === 1) return v[0] === w[0];
      if (w[1] > 0 || want.parts === 2) return v[0] === 0 && v[1] === w[1];
      return v[0] === 0 && v[1] === 0 && v[2] === w[2];
    }
    case '~':
      if (cmp(v, w) < 0) return false;
      return want.parts === 1 ? v[0] === w[0] : v[0] === w[0] && v[1] === w[1];
    default:
      // `1.2.3` is exact; `1.2` and `1` match any version starting so.
      return want.parts === 3 ? cmp(v, w) === 0 : v.slice(0, want.parts).every((x, i) => x === w[i]);
  }
}

/** Splits `id@range` (the range is optional). */
export function parseRequirement(entry: string): { id: string; range: string } {
  const at = entry.indexOf('@');
  return at < 0 ? { id: entry.trim(), range: '*' } : { id: entry.slice(0, at).trim(), range: entry.slice(at + 1).trim() };
}

/** Whether every entry is met by the core plugins a connection reports enabled. */
export function meetsRequirements(entries: readonly string[] | undefined, plugins: readonly PluginInfo[]): boolean {
  return (entries ?? []).every((entry) => {
    const { id, range } = parseRequirement(entry);
    return plugins.some((p) => p.id === id && satisfies(p.version, range));
  });
}
