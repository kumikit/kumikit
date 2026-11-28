import { describe, expect, it } from 'vitest';

import { CoreConnection, backoff } from '../src/connections/connection.svelte';
import { FakeCore, ManualTimers, event, settle } from './fake-core';

const delta = (seq: number, text: string) => event('t1', seq, 'text.delta', { turn: 'u1', text });

async function connected(core = new FakeCore()) {
  const timers = new ManualTimers();
  const c = new CoreConnection('local', core.platform(), { timers });
  await c.start();
  await settle();
  return { core, timers, c };
}

describe('connection', () => {
  it('shakes hands and loads the core state', async () => {
    const core = new FakeCore();
    core.handle('provider.list', () => ({ providers: [{ id: 'a.b/c', name: 'C', plugin: 'a.b' }] }));
    core.handle('workspace.list', () => ({ workspaces: [{ id: 'w1', name: 'proj', root: '/p' }] }));
    const { c } = await connected(core);
    expect(c.status).toBe('ready');
    expect(c.providers.map((p) => p.id)).toEqual(['a.b/c']);
    expect(c.workspaces.map((w) => w.id)).toEqual(['w1']);
    expect(core.requests[0]).toMatchObject({ method: 'core.hello', params: { protocol: '0.13' } });
  });

  it('fans one wire subscription out to every listener', async () => {
    const { core, c } = await connected();
    const a: number[] = [];
    const b: number[] = [];
    c.subscribe('thread:t1', 0, (e) => a.push(e.seq));
    c.subscribe('thread:t1', 0, (e) => b.push(e.seq));
    await settle();
    core.append(delta(1, 'x'));
    core.append(delta(2, 'y'));
    expect(a).toEqual([1, 2]);
    expect(b).toEqual([1, 2]);
    expect(core.requests.filter((r) => r.method === 'events.subscribe')).toHaveLength(1);
  });

  it('replays for a late listener without duplicating for the others', async () => {
    const { core, c } = await connected();
    core.append(delta(1, 'x'));
    const a: number[] = [];
    c.subscribe('thread:t1', 0, (e) => a.push(e.seq));
    await settle();
    core.append(delta(2, 'y'));
    const late: number[] = [];
    c.subscribe('thread:t1', 0, (e) => late.push(e.seq));
    await settle();
    expect(a).toEqual([1, 2]);
    expect(late).toEqual([1, 2]);
  });

  it('resubscribes after a reconnect from the last seq, with no gap or duplicate', async () => {
    const { core, timers, c } = await connected();
    const seen: number[] = [];
    c.subscribe('thread:t1', 0, (e) => seen.push(e.seq));
    await settle();
    core.append(delta(1, 'a'));
    core.drop();
    expect(c.status).toBe('reconnecting');
    // Events the client missed while away.
    core.logs.get('thread:t1')!.push(delta(2, 'b'), delta(3, 'c'));
    core.refuse = true;
    expect(timers.fire()).toBe(500);
    await settle();
    expect(c.status).toBe('reconnecting');
    core.refuse = false;
    expect(timers.fire()).toBe(1000);
    await settle();
    expect(c.status).toBe('ready');
    const resub = core.requests.filter((r) => r.method === 'events.subscribe').at(-1);
    expect(resub?.params).toEqual({ topic: 'thread:t1', after: 1 });
    core.append(delta(4, 'd'));
    expect(seen).toEqual([1, 2, 3, 4]);
  });

  it('backs off from 0.5 s to at most 30 s', () => {
    expect([0, 1, 2, 3, 10].map(backoff)).toEqual([500, 1000, 2000, 4000, 30000]);
  });

  it('keeps thread lists current from the workspace topic', async () => {
    const core = new FakeCore();
    const t = { id: 't1', workspace: 'w1', provider: 'a/b', model: 'm', status: 'idle', created: '1', updated: '1' };
    core.handle('thread.list', () => ({ threads: [t] }));
    const { c } = await connected(core);
    c.watchWorkspace('w1');
    await settle();
    expect(c.threads.w1?.map((x) => x.status)).toEqual(['idle']);
    core.notify('events.event', { topic: 'workspace:w1', seq: 1, time: '2', kind: 'thread.updated', data: { thread: { ...t, status: 'running', updated: '2' } } });
    expect(c.threads.w1?.map((x) => x.status)).toEqual(['running']);
    // An older copy (a late list result) does not win.
    c.noteThread({ ...t, status: 'idle', updated: '1' } as never);
    expect(c.threads.w1?.map((x) => x.status)).toEqual(['running']);
  });

  it('keeps a thread an agent started out of the sidebar list (ADR 0036)', async () => {
    const core = new FakeCore();
    const t = { id: 't1', workspace: 'w1', provider: 'a/b', model: 'm', status: 'running', created: '1', updated: '1' };
    core.handle('thread.list', () => ({ threads: [t] }));
    const { c } = await connected(core);
    c.watchWorkspace('w1');
    await settle();
    // `start_thread` from t1's turn: a side thread whose parent is t1.
    const started = { ...t, id: 't2', status: 'idle', side: true, parent: 't1', created: '2', updated: '2' };
    core.notify('events.event', { topic: 'workspace:w1', seq: 1, time: '2', kind: 'thread.created', data: { thread: started } });
    expect(c.threads.w1?.map((x) => x.id)).toEqual(['t1']);
    expect(c.sideThreads.t2?.parent).toBe('t1');
  });

  it('stops retrying on a protocol mismatch and remembers the core version', async () => {
    const core = new FakeCore();
    core.handle('core.hello', () => ({ error: { code: 'protocol_mismatch', message: 'core speaks protocol 0.4' } }));
    const { c, timers } = await connected(core);
    expect(c.status).toBe('mismatch');
    expect(c.coreProtocol).toBe('0.4');
    expect(timers.pending).toEqual([]);
  });

  it('refreshes providers when plugins change', async () => {
    const { core, c } = await connected();
    core.handle('provider.list', () => ({ providers: [{ id: 'x.y/z', name: 'Z', plugin: 'x.y' }] }));
    core.notify('core.pluginsChanged', { plugins: [{ id: 'x.y', version: '1', contributes: { providers: ['x.y/z'], commands: [], settings: false } }] });
    await settle();
    expect(c.plugins.map((p) => p.id)).toEqual(['x.y']);
    expect(c.providers.map((p) => p.id)).toEqual(['x.y/z']);
  });
});
