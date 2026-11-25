/**
 * Pure helpers for Step 7 composer and transcript polish:
 * slash command menu, `@` file mention parsing, tool call grouping (5+ consecutive calls),
 * per-turn changed files diff counts, and elapsed time formatting.
 */
import type { SlashCommand } from '@kumikit/protocol';
import type { TranscriptItem } from '@kumikit/ui-sdk';

export interface SlashMenuEntry {
  name: string;
  description?: string;
  hint?: string;
  source: 'client' | 'agent';
}

export const CLIENT_COMMANDS: readonly SlashMenuEntry[] = [
  { name: 'model', description: 'Switch the active model', hint: '<model-id>', source: 'client' },
  {
    name: 'later',
    description: 'Schedule a deferred message',
    hint: '15m | 14:30 | after <cmd> -- <msg>',
    source: 'client',
  },
  { name: 'btw', description: 'Ask a side question in the inspector', hint: '<question>', source: 'client' },
  { name: 'fork', description: 'Fork this thread from the latest turn', source: 'client' },
  { name: 'rename', description: 'Rename this thread', hint: '<new title>', source: 'client' },
];

/** Returns matching slash commands when the composer holds a `/prefix` with no space yet. */
export function matchSlashCommands(
  text: string,
  agentCommands: readonly SlashCommand[] | undefined,
): SlashMenuEntry[] {
  if (!text.startsWith('/') || /\s/.test(text)) return [];
  const q = text.slice(1).toLowerCase();
  const seen = new Set<string>();
  const all: SlashMenuEntry[] = [];
  for (const cmd of agentCommands ?? []) {
    const name = cmd.name.replace(/^\/+/, '');
    if (!name || seen.has(name)) continue;
    seen.add(name);
    const entry: SlashMenuEntry = { name, source: 'agent' };
    if (cmd.description) entry.description = cmd.description;
    if (cmd.hint) entry.hint = cmd.hint;
    all.push(entry);
  }
  for (const cmd of CLIENT_COMMANDS) {
    if (seen.has(cmd.name)) continue;
    seen.add(cmd.name);
    all.push(cmd);
  }
  return all.filter((c) => c.name.toLowerCase().includes(q));
}

export interface MentionMatch {
  start: number;
  end: number;
  query: string;
}

/** Finds an active `@query` mention ending at `cursor`, if any. */
export function parseMentionAt(text: string, cursor: number): MentionMatch | undefined {
  const before = text.slice(0, cursor);
  const match = /(^|[\s(])@([^\s@"'`]*)$/.exec(before);
  if (!match) return undefined;
  const query = match[2] ?? '';
  const start = cursor - query.length - 1;
  return { start, end: cursor, query };
}

/** Inserts a workspace-relative path as `@path` (quoting paths containing spaces). */
export function formatMentionPath(path: string): string {
  return /\s/.test(path) ? `@"${path}"` : `@${path}`;
}

export type ToolItem = Extract<TranscriptItem, { kind: 'tool' }>;

export type DisplayEntry =
  | { kind: 'item'; item: TranscriptItem; index: number }
  | { kind: 'tool-group'; items: ToolItem[]; startIndex: number; failed: number; pending: number };

/** Collapses runs of 5+ consecutive tool calls into a single collapsible group. */
export function groupTurnItems(items: readonly TranscriptItem[]): DisplayEntry[] {
  const out: DisplayEntry[] = [];
  let i = 0;
  while (i < items.length) {
    const it = items[i]!;
    if (it.kind !== 'tool') {
      out.push({ kind: 'item', item: it, index: i });
      i++;
      continue;
    }
    let j = i;
    while (j < items.length && items[j]!.kind === 'tool') j++;
    const count = j - i;
    if (count >= 5) {
      const group = items.slice(i, j) as ToolItem[];
      const failed = group.filter((t) => t.ok === false).length;
      const pending = group.filter((t) => t.ok === undefined).length;
      out.push({ kind: 'tool-group', items: group, startIndex: i, failed, pending });
    } else {
      for (let k = i; k < j; k++) out.push({ kind: 'item', item: items[k]!, index: k });
    }
    i = j;
  }
  return out;
}

export interface ChangedFileSummary {
  path: string;
  diff?: string;
  added: number;
  removed: number;
}

/** Collects per-turn `file.changed` entries with unified-diff added/removed line counts. */
export function collectTurnFiles(items: readonly TranscriptItem[]): ChangedFileSummary[] {
  const byPath = new Map<string, ChangedFileSummary>();
  for (const it of items) {
    if (it.kind !== 'file.changed') continue;
    let added = 0;
    let removed = 0;
    if (it.diff) {
      for (const line of it.diff.split('\n')) {
        if (line.startsWith('+++') || line.startsWith('---')) continue;
        if (line.startsWith('+')) added++;
        else if (line.startsWith('-')) removed++;
      }
    }
    const prev = byPath.get(it.path);
    if (prev) {
      prev.added += added;
      prev.removed += removed;
      if (it.diff) prev.diff = prev.diff ? `${prev.diff}\n${it.diff}` : it.diff;
    } else {
      const entry: ChangedFileSummary = { path: it.path, added, removed };
      if (it.diff !== undefined) entry.diff = it.diff;
      byPath.set(it.path, entry);
    }
  }
  return [...byPath.values()];
}

/** Formats elapsed time between ISO timestamps (or from `started` to `nowMs`). */
export function formatElapsed(started: string, ended: string | undefined, nowMs: number): string {
  const s = Date.parse(started);
  const e = ended ? Date.parse(ended) : nowMs;
  if (Number.isNaN(s) || Number.isNaN(e)) return '';
  const diffMs = Math.max(0, e - s);
  if (diffMs < 1000) return `${(diffMs / 1000).toFixed(1)}s`;
  const totalSec = Math.floor(diffMs / 1000);
  if (totalSec < 60) return `${totalSec}s`;
  const min = Math.floor(totalSec / 60);
  const sec = totalSec % 60;
  return `${min}m ${sec}s`;
}
