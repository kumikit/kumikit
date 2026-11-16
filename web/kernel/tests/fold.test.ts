import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import type { EventNotification, ThreadStatus } from '@kumikit/protocol';
import { describe, expect, it } from 'vitest';

import { emptyView, foldEvents } from '../src/threads/fold';

const address = { connection: 'local', workspace: 'w', thread: 't' };
const fixture = JSON.parse(
  readFileSync(
    fileURLToPath(new URL('../../../crates/kumikit-kernel/tests/fixtures/status.json', import.meta.url)),
    'utf8',
  ),
) as { thread: string; cases: { name: string; events: object[]; statuses: ThreadStatus[] }[] };

let time = 0;
function stamp(events: object[], from = 1): EventNotification[] {
  return events.map(
    (e, i) =>
      ({
        topic: `thread:${fixture.thread}`,
        seq: from + i,
        time: `2026-11-16T10:00:${String(time++ % 60).padStart(2, '0')}.000Z`,
        ...e,
      }) as EventNotification,
  );
}

describe('status', () => {
  for (const c of fixture.cases) {
    it(`matches the kernel: ${c.name}`, () => {
      let view = emptyView(address);
      const statuses: ThreadStatus[] = [];
      for (const e of stamp(c.events)) {
        view = foldEvents(view, [e]);
        statuses.push(view.status);
      }
      expect(statuses).toEqual(c.statuses);
    });
  }
});

describe('the transcript', () => {
  const tool = fixture.cases.find((c) => c.name === 'an approval allowed')!;

  it('folds a batch the same as one event at a time', () => {
    const events = stamp(fixture.cases[0]!.events);
    const once = foldEvents(emptyView(address), events);
    const steps = events.reduce((v, e) => foldEvents(v, [e]), emptyView(address));
    expect(once).toEqual(steps);
    expect(once.turns[0]!.items).toEqual([{ kind: 'message', text: 'Hello', done: true }]);
  });

  it('skips events it has already folded', () => {
    const events = stamp(fixture.cases[0]!.events);
    const view = foldEvents(emptyView(address), events);
    expect(foldEvents(view, events)).toBe(view);
    expect(foldEvents(view, [...events.slice(2), ...events])).toBe(view);
  });

  it('never changes the view it was given', () => {
    const events = stamp(tool.events);
    const half = foldEvents(emptyView(address), events.slice(0, 3));
    const snapshot = structuredClone(half);
    const full = foldEvents(half, events.slice(3));
    expect(half).toEqual(snapshot);
    expect(full).not.toBe(half);
    expect(full.turns[0]).not.toBe(half.turns[0]);
  });

  it('tracks pending approvals and their resolution', () => {
    const events = stamp(tool.events);
    const waiting = foldEvents(emptyView(address), events.slice(0, 3));
    expect(waiting.pendingApprovals.map((a) => a.id)).toEqual(['a1']);
    const done = foldEvents(waiting, events.slice(3));
    expect(done.pendingApprovals).toEqual([]);
    const approval = done.turns[0]!.items.find((i) => i.kind === 'approval');
    expect(approval).toMatchObject({ kind: 'approval', resolution: 'allow' });
    expect(done.turns[0]!.outcome).toBe('done');
  });

  it('collects tool output, results, files, usage and plugin events', () => {
    const t = 'u1';
    const events = stamp([
      { kind: 'turn.started', data: { turn: t, input: { text: 'go' } } },
      { kind: 'reasoning.delta', data: { turn: t, text: 'thin' } },
      { kind: 'reasoning.delta', data: { turn: t, text: 'king' } },
      { kind: 'tool.started', data: { turn: t, call: 'c1', name: 'Bash', input: { cmd: 'ls' } } },
      { kind: 'tool.output', data: { turn: t, call: 'c1', chunk: 'a\n' } },
      { kind: 'tool.output', data: { turn: t, call: 'c1', chunk: 'b\n' } },
      { kind: 'tool.finished', data: { turn: t, call: 'c1', ok: true, result: { code: 0 } } },
      { kind: 'file.changed', data: { turn: t, path: 'a.txt', diff: '+x' } },
      { kind: 'usage', data: { turn: t, inputTokens: 10, outputTokens: 3, cost: 0.01 } },
      { kind: 'plugin.event', data: { plugin: 'p.q', kind: 'acp.plan', data: { steps: 2 } } },
      { kind: 'future.kind', data: {} },
      { kind: 'turn.ended', data: { turn: t, outcome: 'failed', error: 'boom' } },
    ]);
    const view = foldEvents(emptyView(address), events);
    const turn = view.turns[0]!;
    expect(turn.items).toEqual([
      { kind: 'reasoning', text: 'thinking' },
      { kind: 'tool', call: 'c1', name: 'Bash', input: { cmd: 'ls' }, output: 'a\nb\n', ok: true, result: { code: 0 } },
      { kind: 'file.changed', path: 'a.txt', diff: '+x' },
      { kind: 'plugin.event', plugin: 'p.q', event: 'acp.plan', data: { steps: 2 } },
    ]);
    expect(turn.usage).toEqual({ inputTokens: 10, outputTokens: 3, cost: 0.01 });
    expect(turn).toMatchObject({ outcome: 'failed', error: 'boom' });
    expect(view.status).toBe('failed');
    expect(view.seq).toBe(12);
  });

  it('drops a queued message when its turn starts', () => {
    const view = { ...emptyView(address), queued: ['next', 'later'] };
    const after = foldEvents(view, stamp([{ kind: 'turn.started', data: { turn: 'u2', input: { text: 'next' } } }]));
    expect(after.queued).toEqual(['later']);
  });
});
