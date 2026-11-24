import { describe, expect, it } from 'vitest';

import type { ThreadView } from '@kumikit/ui-sdk';
import { CoreConnection } from '../src/connections/connection.svelte';
import { ThreadStore } from '../src/threads/store.svelte';
import { FakeCore, ManualTimers, event, settle } from './fake-core';

const address = { connection: 'local', workspace: 'w1', thread: 't1' };

async function setup() {
  const core = new FakeCore();
  const c = new CoreConnection('local', core.platform(), { timers: new ManualTimers() });
  await c.start();
  await settle();
  const frames: (() => void)[] = [];
  const store = new ThreadStore(() => c, (fn) => frames.push(fn));
  const runFrame = () => frames.splice(0).forEach((f) => f());
  return { core, c, store, runFrame };
}

describe('thread store', () => {
  it('folds events once per frame and tells every watcher', async () => {
    const { core, store, runFrame } = await setup();
    const a: ThreadView[] = [];
    const b: ThreadView[] = [];
    store.watch(address, (v) => a.push(v));
    store.watch(address, (v) => b.push(v));
    await settle();
    core.append(event('t1', 1, 'turn.started', { turn: 'u1', input: { text: 'hi' } }));
    core.append(event('t1', 2, 'text.delta', { turn: 'u1', text: 'He' }));
    core.append(event('t1', 3, 'text.delta', { turn: 'u1', text: 'llo' }));
    expect(a).toHaveLength(1);
    runFrame();
    expect(a).toHaveLength(2);
    expect(a[1]!.seq).toBe(3);
    expect(a[1]!.status).toBe('running');
    expect(a[1]!.turns[0]!.items).toEqual([{ kind: 'message', text: 'Hello', done: false }]);
    expect(b.at(-1)).toBe(a.at(-1));
    expect(core.requests.filter((r) => r.method === 'events.subscribe')).toHaveLength(1);
  });

  it('shows the queue from the log until its message starts', async () => {
    const { core, store, runFrame } = await setup();
    core.handle('thread.send', () => ({ turn: 'u2', queued: true, entry: 'q1' }));
    let view!: ThreadView;
    store.watch(address, (v) => (view = v));
    await settle();
    await store.send(address, 'next');
    expect(view.queue).toEqual([]);
    core.append(event('t1', 1, 'queue.added', { entry: { id: 'q1', turn: 'u2', input: { text: 'next' } } }));
    runFrame();
    expect(view.queued).toEqual(['next']);
    core.append(event('t1', 2, 'turn.started', { turn: 'u2', input: { text: 'next' }, entry: 'q1' }));
    runFrame();
    expect(view.queue).toEqual([]);
  });

  it('edits the queue through the core, keeping attachments', async () => {
    const { core, store, runFrame } = await setup();
    const image = { name: 'a.png', mimeType: 'image/png', data: 'AA==' };
    let view!: ThreadView;
    store.watch(address, (v) => (view = v));
    await settle();
    core.append(event('t1', 1, 'queue.added', { entry: { id: 'q1', turn: 'u2', input: { text: 'old', attachments: [image] } } }));
    runFrame();
    expect(view.queue).toHaveLength(1);
    for (const m of ['thread.queue.update', 'thread.queue.remove', 'thread.queue.move', 'thread.queue.sendNow', 'thread.queue.resume']) {
      core.handle(m, () => ({}));
    }
    await store.queueUpdate(address, 'q1', 'new');
    await store.queueMove(address, 'q1');
    await store.queueMove(address, 'q1', 'q0');
    await store.queueSendNow(address, 'q1');
    await store.queueRemove(address, 'q1');
    await store.queueResume(address);
    const calls = core.requests.filter((r) => r.method.startsWith('thread.queue.'));
    expect(calls.map((r) => [r.method, r.params])).toEqual([
      ['thread.queue.update', { thread: 't1', entry: 'q1', input: { text: 'new', attachments: [image] } }],
      ['thread.queue.move', { thread: 't1', entry: 'q1' }],
      ['thread.queue.move', { thread: 't1', entry: 'q1', before: 'q0' }],
      ['thread.queue.sendNow', { thread: 't1', entry: 'q1' }],
      ['thread.queue.remove', { thread: 't1', entry: 'q1' }],
      ['thread.queue.resume', { thread: 't1' }],
    ]);
  });

  it('stops following a thread nobody watches', async () => {
    const { core, store } = await setup();
    const stop = store.watch(address, () => {});
    await settle();
    stop();
    await settle();
    expect(core.requests.at(-1)).toMatchObject({ method: 'events.unsubscribe', params: { topic: 'thread:t1' } });
  });
});
