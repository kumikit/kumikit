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

/** Item kinds that stay visible in a finished turn: they record what the user decided or said. */
const USER_KINDS: ReadonlySet<TranscriptItem['kind']> = new Set(['approval', 'steer']);

export interface IndexedItem {
  item: TranscriptItem;
  index: number;
}

/** A finished turn split into its collapsed activity, the user's own items and the final reply. */
export interface TurnSummaryParts {
  /** Agent work (tools, intermediate messages, plugin events), collapsed by default. */
  activity: IndexedItem[];
  /** Approvals and steers, kept visible. */
  user: IndexedItem[];
  /** The trailing assistant message(s): the turn's answer. */
  summary: IndexedItem[];
  /** All of the turn's reasoning, in order; shown in the Thinking box, never in the summary. */
  thinking: string;
  tools: number;
  failed: number;
}

function lastToolIndex(items: readonly TranscriptItem[]): number {
  for (let i = items.length - 1; i >= 0; i--) {
    if (items[i]!.kind === 'tool') return i;
  }
  return -1;
}

/**
 * Whether `item` at `index` belongs in the Thinking box rather than the tool/activity stream:
 * explicit `reasoning` items plus intermediate narration messages emitted before a tool call.
 */
function isThinkingItem(item: TranscriptItem, index: number, lastTool: number): boolean {
  if (item.kind === 'reasoning') return true;
  return item.kind === 'message' && index < lastTool;
}

/**
 * The turn's thinking as one text: every reasoning item and pre-tool narration message,
 * in order, one blank line apart.
 */
export function turnThinking(items: readonly TranscriptItem[]): string {
  const lastTool = lastToolIndex(items);
  return items
    .flatMap((it, index) => (isThinkingItem(it, index, lastTool) ? [(it as { text: string }).text] : []))
    .filter((t) => t.trim().length > 0)
    .join('\n\n');
}

/** The turn's items without thinking items: those are shown in the Thinking box instead. */
export function withoutReasoning(items: readonly TranscriptItem[]): TranscriptItem[] {
  const lastTool = lastToolIndex(items);
  return items.filter((it, index) => !isThinkingItem(it, index, lastTool));
}

/**
 * Splits a finished turn for the summary card. The summary is the run of `message` items at
 * the end of the turn (ignoring `file.changed` and `reasoning`); the agent's tools and other
 * work go to `activity`; reasoning and pre-tool narration go to `thinking`.
 */
export function splitFinishedTurn(items: readonly TranscriptItem[]): TurnSummaryParts {
  let start = items.length;
  for (let i = items.length - 1; i >= 0; i--) {
    const kind = items[i]!.kind;
    if (kind === 'file.changed' || kind === 'reasoning') continue;
    if (kind !== 'message') break;
    start = i;
  }
  const lastTool = lastToolIndex(items);
  const parts: TurnSummaryParts = {
    activity: [],
    user: [],
    summary: [],
    thinking: turnThinking(items),
    tools: 0,
    failed: 0,
  };
  items.forEach((item, index) => {
    if (item.kind === 'file.changed' || isThinkingItem(item, index, lastTool)) return;
    if (item.kind === 'message' && index >= start) parts.summary.push({ item, index });
    else if (USER_KINDS.has(item.kind)) parts.user.push({ item, index });
    else {
      parts.activity.push({ item, index });
      if (item.kind === 'tool') {
        parts.tools++;
        if (item.ok === false) parts.failed++;
      }
    }
  });
  return parts;
}

/** The activity header text: "Used 3 tools", "Used 1 tool · 1 failed", "2 steps", or "Thinking". */
export function activityLabel(parts: Pick<TurnSummaryParts, 'activity' | 'tools' | 'failed'>): string {
  const n = parts.activity.length;
  if (n === 0) return 'Thinking';
  const base =
    parts.tools > 0
      ? `Used ${parts.tools} ${parts.tools === 1 ? 'tool' : 'tools'}`
      : `${n} ${n === 1 ? 'step' : 'steps'}`;
  return parts.failed > 0 ? `${base} · ${parts.failed} failed` : base;
}

/** Totals across changed files. */
export function totalChanges(files: readonly ChangedFileSummary[]): { added: number; removed: number } {
  let added = 0;
  let removed = 0;
  for (const f of files) {
    added += f.added;
    removed += f.removed;
  }
  return { added, removed };
}

/**
 * A GitHub-style change bar of `blocks` cells: added cells, then removed, then empty. Any
 * non-zero side gets at least one cell; with nothing changed every cell is empty.
 */
export function changeBar(added: number, removed: number, blocks = 5): ('add' | 'del' | 'none')[] {
  const total = added + removed;
  if (total === 0) return Array.from({ length: blocks }, () => 'none' as const);
  const filled = Math.min(blocks, total);
  let add = Math.round((added / total) * filled);
  if (added > 0 && add === 0) add = 1;
  if (removed > 0 && add === filled) add = filled - 1;
  const del = filled - add;
  return [
    ...Array.from({ length: add }, () => 'add' as const),
    ...Array.from({ length: del }, () => 'del' as const),
    ...Array.from({ length: blocks - filled }, () => 'none' as const),
  ];
}

export interface DiffRow {
  kind: 'hunk' | 'add' | 'del' | 'ctx';
  text: string;
  oldNo?: number;
  newNo?: number;
}

/**
 * Parses unified diff text into display rows with line numbers. File headers (`---`, `+++`,
 * `diff`, `index`) are dropped; a diff without `@@` headers is numbered from line 1.
 */
export function parseUnifiedDiff(diff: string): DiffRow[] {
  const rows: DiffRow[] = [];
  let oldNo = 1;
  let newNo = 1;
  const lines = diff.split('\n');
  if (lines.length > 0 && lines[lines.length - 1] === '') lines.pop();
  for (const line of lines) {
    if (/^(---|\+\+\+|diff |index |new file mode|deleted file mode|similarity |rename )/.test(line)) continue;
    const hunk = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@/.exec(line);
    if (hunk) {
      oldNo = Number(hunk[1]);
      newNo = Number(hunk[2]);
      rows.push({ kind: 'hunk', text: line });
    } else if (line.startsWith('\\')) {
      continue; // "\ No newline at end of file"
    } else if (line.startsWith('+')) {
      rows.push({ kind: 'add', text: line.slice(1), newNo: newNo++ });
    } else if (line.startsWith('-')) {
      rows.push({ kind: 'del', text: line.slice(1), oldNo: oldNo++ });
    } else {
      rows.push({ kind: 'ctx', text: line.startsWith(' ') ? line.slice(1) : line, oldNo: oldNo++, newNo: newNo++ });
    }
  }
  return rows;
}

export type SplitRow = { kind: 'hunk'; text: string } | { kind: 'pair'; left?: DiffRow; right?: DiffRow };

/** Pairs removed and added lines side by side for the split view; context lines sit on both sides. */
export function splitDiffRows(rows: readonly DiffRow[]): SplitRow[] {
  const out: SplitRow[] = [];
  let dels: DiffRow[] = [];
  let adds: DiffRow[] = [];
  const flush = () => {
    const n = Math.max(dels.length, adds.length);
    for (let i = 0; i < n; i++) {
      const pair: SplitRow = { kind: 'pair' };
      if (dels[i]) pair.left = dels[i]!;
      if (adds[i]) pair.right = adds[i]!;
      out.push(pair);
    }
    dels = [];
    adds = [];
  };
  for (const row of rows) {
    if (row.kind === 'del') {
      if (adds.length > 0) flush();
      dels.push(row);
    } else if (row.kind === 'add') adds.push(row);
    else {
      flush();
      out.push(row.kind === 'hunk' ? { kind: 'hunk', text: row.text } : { kind: 'pair', left: row, right: row });
    }
  }
  flush();
  return out;
}
