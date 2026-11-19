/**
 * Lockfiles in the UI (10-distribution, Lockfiles): reading one to import it, and adding
 * the client's trust and enabled flags when exporting the app's lockfile.
 *
 * Lockfiles are written by Kumikit (`kumikit_protocol::lockfile`), so this reads only the
 * small part of TOML that format uses: comments, `key = value` with strings, booleans,
 * integers and arrays of strings, `[[plugin]]` and `[plugin.granted]`. Anything else is
 * refused rather than guessed.
 */
import type { Permissions } from '@kumikit/protocol';

export interface LockEntry {
  id: string;
  version: string;
  sha256: string;
  /** `registry+<url>`, `file` or `bundled`. */
  source: string;
  key?: string;
  state?: string;
  granted?: Permissions;
  enabled?: boolean;
  trusted?: boolean;
}

export interface Lock {
  format: number;
  plugins: LockEntry[];
}

type Value = string | number | boolean | string[];

function parseValue(raw: string, line: number): Value {
  const v = raw.trim();
  if (v === 'true' || v === 'false') return v === 'true';
  if (/^-?\d+$/.test(v)) return Number(v);
  if (v.startsWith('"')) return parseString(v, line).value;
  if (v.startsWith('[') && v.endsWith(']')) {
    const inner = v.slice(1, -1).trim();
    const out: string[] = [];
    let rest = inner;
    while (rest.length > 0) {
      const s = parseString(rest, line);
      out.push(s.value);
      rest = s.rest.trim();
      if (rest.startsWith(',')) rest = rest.slice(1).trim();
      else if (rest.length > 0) throw new Error(`line ${line}: expected a comma in the list`);
    }
    return out;
  }
  throw new Error(`line ${line}: a value Kumikit lockfiles do not use`);
}

function parseString(text: string, line: number): { value: string; rest: string } {
  if (!text.startsWith('"')) throw new Error(`line ${line}: expected a string`);
  let out = '';
  for (let i = 1; i < text.length; i++) {
    const c = text[i]!;
    if (c === '"') return { value: out, rest: text.slice(i + 1) };
    if (c === '\\') {
      const n = text[++i];
      const map: Record<string, string> = { '"': '"', '\\': '\\', n: '\n', t: '\t' };
      if (n === undefined || !(n in map)) throw new Error(`line ${line}: an escape Kumikit lockfiles do not use`);
      out += map[n];
    } else out += c;
  }
  throw new Error(`line ${line}: an unterminated string`);
}

const PERMISSION_KEYS = new Set(['spawn', 'network', 'files', 'secrets', 'tools']);

/** Reads a Kumikit lockfile (format 1). Throws with a line number on anything else. */
export function parseLock(text: string): Lock {
  let format: number | undefined;
  const plugins: Record<string, Value | Record<string, Value>>[] = [];
  let section: 'top' | 'plugin' | 'granted' = 'top';
  text.split('\n').forEach((rawLine, i) => {
    const line = i + 1;
    const l = rawLine.trim();
    if (l === '' || l.startsWith('#')) return;
    if (l === '[[plugin]]') {
      plugins.push({});
      section = 'plugin';
      return;
    }
    if (l === '[plugin.granted]') {
      if (plugins.length === 0) throw new Error(`line ${line}: [plugin.granted] before any [[plugin]]`);
      plugins.at(-1)!.granted = {};
      section = 'granted';
      return;
    }
    if (l.startsWith('[')) throw new Error(`line ${line}: unknown section ${l}`);
    const eq = l.indexOf('=');
    if (eq < 0) throw new Error(`line ${line}: expected key = value`);
    const key = l.slice(0, eq).trim();
    const value = parseValue(l.slice(eq + 1), line);
    if (section === 'top') {
      if (key !== 'format' || typeof value !== 'number') throw new Error(`line ${line}: unknown key ${key}`);
      format = value;
    } else if (section === 'plugin') {
      plugins.at(-1)![key] = value;
    } else {
      if (!PERMISSION_KEYS.has(key)) throw new Error(`line ${line}: unknown permission ${key}`);
      (plugins.at(-1)!.granted as Record<string, Value>)[key] = value;
    }
  });
  if (format !== 1) throw new Error(format === undefined ? 'not a Kumikit lockfile (no format)' : `the lockfile has format ${format}; this Kumikit reads format 1`);
  const str = (o: Record<string, unknown>, k: string): string => {
    if (typeof o[k] !== 'string') throw new Error(`an entry has no ${k}`);
    return o[k] as string;
  };
  return {
    format,
    plugins: plugins.map((p): LockEntry => {
      const entry: LockEntry = { id: str(p, 'id'), version: str(p, 'version'), sha256: str(p, 'sha256'), source: str(p, 'source') };
      if (typeof p.key === 'string') entry.key = p.key;
      if (typeof p.state === 'string') entry.state = p.state;
      if (typeof p.enabled === 'boolean') entry.enabled = p.enabled;
      if (typeof p.trusted === 'boolean') entry.trusted = p.trusted;
      const g = p.granted as Record<string, Value> | undefined;
      if (g) {
        const list = (k: string) => (Array.isArray(g[k]) ? (g[k] as string[]) : []);
        entry.granted = {
          spawn: list('spawn'),
          network: list('network'),
          files: g.files === 'read_workspace' || g.files === 'workspace' ? g.files : 'none',
          secrets: list('secrets'),
          tools: g.tools === true,
        };
      }
      return entry;
    }),
  };
}

/** The registry's index URL of an entry installed from one. */
export function registryOf(e: LockEntry): string | undefined {
  return e.source.startsWith('registry+') ? e.source.slice('registry+'.length) : undefined;
}

/** Adds `enabled` and `trusted` to each entry of the app's lockfile (the shell writes the
 * rest): the line after an entry's `id` gets them. */
export function withClientFlags(text: string, flags: (id: string) => { enabled: boolean; trusted: boolean }): string {
  return text
    .split('\n')
    .flatMap((line) => {
      const m = /^id = "([^"\\]+)"$/.exec(line);
      if (!m) return [line];
      const f = flags(m[1]!);
      return [line, `enabled = ${f.enabled}`, `trusted = ${f.trusted}`];
    })
    .join('\n');
}
