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
  const store = new ThreadStore(c, (fn) => frames.push(fn));
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

  it('shows a message sent during a turn as queued until its turn starts', async () => {
    const { core, store, runFrame } = await setup();
    core.handle('thread.send', () => ({ turn: 'u2', queued: true }));
    let view!: ThreadView;
    store.watch(address, (v) => (view = v));
    await store.send(address, 'next');
    expect(view.queued).toEqual(['next']);
    core.append(event('t1', 1, 'turn.started', { turn: 'u2', input: { text: 'next' } }));
    runFrame();
    expect(view.queued).toEqual([]);
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
