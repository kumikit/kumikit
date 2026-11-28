import type { TranscriptItem } from '@kumikit/ui-sdk';
import { describe, expect, it } from 'vitest';

import {
  activityLabel,
  changeBar,
  parseUnifiedDiff,
  splitDiffRows,
  splitFinishedTurn,
  totalChanges,
  turnThinking,
  withoutReasoning,
} from '../src/transcript';

const tool = (ok?: boolean): TranscriptItem => ({ kind: 'tool', call: 'c', name: 'Read', input: {}, ...(ok === undefined ? {} : { ok }) });
const msg = (text: string): TranscriptItem => ({ kind: 'message', text, done: true });

describe('splitFinishedTurn', () => {
  it('puts the trailing messages in the summary and the rest in collapsed activity', () => {
    const items: TranscriptItem[] = [
      msg('Let me look.'),
      tool(true),
      { kind: 'reasoning', text: 'hmm' },
      tool(false),
      { kind: 'approval' } as unknown as TranscriptItem,
      msg('Done.'),
      { kind: 'file.changed', path: 'a.ts' },
      msg('Also this.'),
    ];
    const parts = splitFinishedTurn(items);
    expect(parts.summary.map((s) => s.index)).toEqual([5, 7]);
    expect(parts.activity.map((s) => s.index)).toEqual([1, 3]);
    expect(parts.thinking).toBe('Let me look.\n\nhmm');
    expect(parts.user.map((s) => s.index)).toEqual([4]);
    expect(parts.tools).toBe(2);
    expect(parts.failed).toBe(1);
    expect(activityLabel(parts)).toBe('Used 2 tools · 1 failed');
  });

  it('keeps the trailing summary even when a reasoning item follows it', () => {
    const items: TranscriptItem[] = [tool(true), msg('Final answer.'), { kind: 'reasoning', text: 'afterthought' }];
    const parts = splitFinishedTurn(items);
    expect(parts.summary.map((s) => s.index)).toEqual([1]);
    expect(parts.activity.map((s) => s.index)).toEqual([0]);
    expect(parts.thinking).toBe('afterthought');
  });

  it('has no summary when the turn ends on a tool, and labels steps without tools', () => {
    expect(splitFinishedTurn([msg('a'), tool(true)]).summary).toEqual([]);
    const parts = splitFinishedTurn([{ kind: 'reasoning', text: 'r' }, msg('answer')]);
    expect(parts.summary.map((s) => s.index)).toEqual([1]);
    expect(parts.activity).toEqual([]);
    expect(activityLabel(parts)).toBe('Thinking');
    expect(activityLabel(splitFinishedTurn([msg('a'), tool(true), msg('b')]))).toBe('Used 1 tool');
    expect(activityLabel(splitFinishedTurn([tool(true), msg('ok')]))).toBe('Used 1 tool');
  });
});

describe('thinking', () => {
  it('joins reasoning and pre-tool narration in order and keeps them out of the tool list', () => {
    const items: TranscriptItem[] = [
      { kind: 'reasoning', text: 'first' },
      msg('Let me check a.ts.'),
      tool(true),
      { kind: 'reasoning', text: 'second' },
      msg('Now checking b.ts.'),
      tool(true),
      msg('answer'),
    ];
    expect(turnThinking(items)).toBe('first\n\nLet me check a.ts.\n\nsecond\n\nNow checking b.ts.');
    expect(withoutReasoning(items).map((i) => i.kind)).toEqual(['tool', 'tool', 'message']);
    expect(turnThinking([msg('x')])).toBe('');
  });
});

describe('change totals and bar', () => {
  it('sums files and scales the bar, keeping a cell for each non-zero side', () => {
    expect(totalChanges([{ path: 'a', added: 3, removed: 1 }, { path: 'b', added: 2, removed: 0 }])).toEqual({ added: 5, removed: 1 });
    expect(changeBar(89, 48)).toEqual(['add', 'add', 'add', 'del', 'del']);
    expect(changeBar(100, 1)).toEqual(['add', 'add', 'add', 'add', 'del']);
    expect(changeBar(1, 100)).toEqual(['add', 'del', 'del', 'del', 'del']);
    expect(changeBar(2, 0)).toEqual(['add', 'add', 'none', 'none', 'none']);
    expect(changeBar(0, 0)).toEqual(['none', 'none', 'none', 'none', 'none']);
  });
});

describe('diff parsing', () => {
  const diff = [
    '--- a/x.ts',
    '+++ b/x.ts',
    '@@ -10,3 +10,4 @@ fn',
    ' keep',
    '-old',
    '+new',
    '+more',
    ' tail',
    '\\ No newline at end of file',
    '',
  ].join('\n');

  it('numbers old and new lines from the hunk header and drops file headers', () => {
    const rows = parseUnifiedDiff(diff);
    expect(rows).toEqual([
      { kind: 'hunk', text: '@@ -10,3 +10,4 @@ fn' },
      { kind: 'ctx', text: 'keep', oldNo: 10, newNo: 10 },
      { kind: 'del', text: 'old', oldNo: 11 },
      { kind: 'add', text: 'new', newNo: 11 },
      { kind: 'add', text: 'more', newNo: 12 },
      { kind: 'ctx', text: 'tail', oldNo: 12, newNo: 13 },
    ]);
  });

  it('numbers a headerless diff from line 1', () => {
    expect(parseUnifiedDiff('-a\n+b')).toEqual([
      { kind: 'del', text: 'a', oldNo: 1 },
      { kind: 'add', text: 'b', newNo: 1 },
    ]);
  });

  it('pairs removals with additions for the split view', () => {
    const split = splitDiffRows(parseUnifiedDiff(diff));
    expect(split.map((r) => (r.kind === 'hunk' ? 'hunk' : `${r.left?.text ?? '_'}|${r.right?.text ?? '_'}`))).toEqual([
      'hunk',
      'keep|keep',
      'old|new',
      '_|more',
      'tail|tail',
    ]);
  });
});
