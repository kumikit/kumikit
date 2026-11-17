// @vitest-environment jsdom
/** Several cores in one window (06-client, Multiple cores; the M3 exit criteria). */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { ThreadView } from '@kumikit/ui-sdk';
import { Kernel } from '../src/app.svelte';
import { FakeCore, ManualTimers, event, platformOf, settle } from './fake-core';

/** Two fake cores that hold the same workspace and thread ids. */
function twoCores() {
  const local = new FakeCore();
  const devbox = new FakeCore();
  for (const core of [local, devbox]) {
    core.handle('workspace.list', () => ({ workspaces: [{ id: 'w1', name: 'proj', root: '/p' }] }));
    core.handle('thread.list', () => ({
      threads: [{ id: 't1', workspace: 'w1', title: 'T', provider: 'a/b', model: 'm', status: 'idle', created: '2026-11-17T10:00:00.000Z', updated: '2026-11-17T10:00:00.000Z' }],
    }));
  }
  const timers = new ManualTimers();
  const platform = platformOf({ local, devbox }, { devbox: 'Dev box' });
  const kernel = new Kernel({ platform, timers });
  return { local, devbox, timers, kernel, platform };
}

async function running() {
  const t = twoCores();
  await t.kernel.start();
  await settle();
  return t;
}

beforeEach(() => {
  vi.stubGlobal('fetch', async () => new Response('{}'));
});
afterEach(() => {
  vi.unstubAllGlobals();
});

const delta = (seq: number, text: string) => event('t1', seq, 'text.delta', { turn: 'u1', text });
const started = (thread: string) => event(thread, 1, 'turn.started', { turn: 'u1', input: { text: 'go' } });

describe('several cores', () => {
  it('connects to every profile, the home one first', async () => {
    const { kernel } = await running();
    expect(kernel.connections.map((c) => [c.id, c.label, c.kind, c.status])).toEqual([
      ['local', 'This computer', 'local', 'ready'],
      ['devbox', 'Dev box', 'tls', 'ready'],
    ]);
  });

  it('keeps the same ids on two cores apart', async () => {
    const { kernel, local, devbox } = await running();
    kernel.openTab('local', 'w1', 't1');
    kernel.openTab('devbox', 'w1', 't1');
    expect(kernel.tabs.map((t) => t.connection)).toEqual(['local', 'devbox']);
    const views: Record<string, ThreadView[]> = { local: [], devbox: [] };
    kernel.threads.watch({ connection: 'local', workspace: 'w1', thread: 't1' }, (v) => views.local!.push(v));
    kernel.threads.watch({ connection: 'devbox', workspace: 'w1', thread: 't1' }, (v) => views.devbox!.push(v));
    await settle();
    local.append(started('t1'));
    devbox.append(started('t1'));
    local.append(delta(2, 'from local'));
    devbox.append(delta(2, 'from devbox'));
    await new Promise((r) => requestAnimationFrame(() => r(undefined)));
    expect(views.local!.at(-1)!.turns[0]!.items).toEqual([{ kind: 'message', text: 'from local', done: false }]);
    expect(views.devbox!.at(-1)!.turns[0]!.items).toEqual([{ kind: 'message', text: 'from devbox', done: false }]);
    // Each core was asked for its own workspace's threads.
    expect(local.requests.some((r) => r.method === 'thread.list')).toBe(true);
    expect(devbox.requests.some((r) => r.method === 'thread.list')).toBe(true);
  });

  it('a core that drops reconnects alone; the other keeps streaming', async () => {
    const { kernel, local, devbox, timers } = await running();
    const views: ThreadView[] = [];
    kernel.threads.watch({ connection: 'local', workspace: 'w1', thread: 't1' }, (v) => views.push(v));
    await settle();
    devbox.drop('the core stopped answering');
    await settle();
    expect(kernel.connectionFor('devbox')!.status).toBe('reconnecting');
    expect(kernel.connectionFor('local')!.status).toBe('ready');
    // The local thread still streams.
    local.append(started('t1'));
    local.append(delta(2, 'still'));
    local.append(delta(3, ' here'));
    await new Promise((r) => requestAnimationFrame(() => r(undefined)));
    expect(views.at(-1)!.turns[0]!.items).toEqual([{ kind: 'message', text: 'still here', done: false }]);
    // Only the dropped core waits to retry.
    expect(timers.pending).toHaveLength(1);
    // Reconnect now skips the backoff.
    kernel.connectionFor('devbox')!.reconnectNow();
    await settle();
    expect(kernel.connectionFor('devbox')!.status).toBe('ready');
    expect(timers.pending).toHaveLength(0);
  });

  it('stops retrying a core that refuses this device or changed its certificate', async () => {
    const { kernel, devbox, timers } = await running();
    for (const code of ['unauthorized', 'certificate_changed'] as const) {
      devbox.refuseWith = code;
      devbox.drop();
      await settle();
      if (timers.pending.length) timers.fire();
      await settle();
      const c = kernel.connectionFor('devbox')!;
      expect(c.status).toBe(code);
      expect(timers.pending).toHaveLength(0);
      devbox.refuseWith = undefined;
      c.reconnectNow();
      await settle();
      expect(c.status).toBe('ready');
    }
  });

  it('routes commands to the active tab’s core', async () => {
    const { kernel, local, devbox } = await running();
    devbox.handle('thread.respondApproval', () => ({}));
    kernel.openTab('devbox', 'w1', 't1');
    await kernel.commands.run('approval.respond', { approval: 'a1', decision: 'allow' });
    expect(devbox.requests.at(-1)).toMatchObject({ method: 'thread.respondApproval', params: { approval: 'a1' } });
    expect(local.requests.some((r) => r.method === 'thread.respondApproval')).toBe(false);
    devbox.handle('thread.send', () => ({ turn: 'u2', queued: false }));
    await kernel.commands.run('thread.send', { address: { connection: 'devbox', workspace: 'w1', thread: 't1' }, text: 'hi' });
    expect(devbox.requests.at(-1)).toMatchObject({ method: 'thread.send', params: { thread: 't1' } });
  });

  it('gates UI plugin contributions on requires.core per tab', async () => {
    const { kernel, devbox } = await running();
    devbox.notify('core.pluginsChanged', { plugins: [{ id: 'kumikit.git', version: '1.2.0', contributes: { providers: [], commands: [], settings: false } }] });
    await settle();
    kernel.clientPlugins.requiresOf = (id: string) => (id === 'test.git-ui' ? ['kumikit.git@^1'] : undefined);
    const mount = () => () => {};
    kernel.slots.register('test.git-ui', 'thread.header', { id: 'git', mount });
    kernel.slots.register('test.plain', 'thread.header', { id: 'plain', mount });
    kernel.commands.register({ id: 'git.status', title: 'Git status' }, () => {}, 'test.git-ui');
    const ids = (c: string) => kernel.slotsIn('thread.header', c).map((e) => e.contribution.id);
    expect(ids('local')).toEqual(['plain']);
    expect(ids('devbox')).toEqual(['git', 'plain']);
    kernel.openTab('local', 'w1');
    expect(kernel.commands.search('git', kernel.when, kernel.commandAllowed).map((c) => c.id)).not.toContain('git.status');
    kernel.openTab('devbox', 'w1');
    expect(kernel.commands.search('git', kernel.when, kernel.commandAllowed).map((c) => c.id)).toContain('git.status');
  });

  it('installs a core plugin on the chosen core, by upload when it is elsewhere', async () => {
    const { kernel, local, devbox } = await running();
    devbox.handle('plugin.install', () => ({ plugin: { id: 'a.b' } }));
    devbox.handle('plugin.list', () => ({ plugins: [] }));
    kernel.corePlugins.choose('devbox');
    expect(kernel.corePlugins.uploads).toBe(true);
    await kernel.corePlugins.upload(new Uint8Array([1, 2, 3, 250]));
    expect(devbox.requests.find((r) => r.method === 'plugin.install')!.params).toEqual({ package: 'AQID+g==' });
    expect(local.requests.some((r) => r.method === 'plugin.install')).toBe(false);
    kernel.corePlugins.choose('local');
    expect(kernel.corePlugins.uploads).toBe(false);
  });

  it('removing a core keeps its tabs, saying it was removed', async () => {
    const { kernel } = await running();
    kernel.openTab('devbox', 'w1');
    expect(await kernel.removeCore('devbox')).toBe(true);
    expect(kernel.connections.map((c) => c.id)).toEqual(['local']);
    expect(kernel.tabs.map((t) => t.connection)).toEqual(['devbox']);
    expect(kernel.connectionFor('devbox')).toBeUndefined();
    // The home core cannot be removed.
    expect(await kernel.removeCore('local')).toBe(false);
  });

  it('adds a core the shell paired', async () => {
    const vm = new FakeCore();
    const kernel = new Kernel({ platform: platformOf({ local: new FakeCore(), vm }, {}, ['local']), timers: new ManualTimers() });
    await kernel.start();
    expect(kernel.connections.map((c) => c.id)).toEqual(['local']);
    const c = kernel.addedCore({ name: 'vm', label: 'VM', kind: 'ssh' });
    await settle();
    expect(c.status).toBe('ready');
    expect(kernel.addedCore({ name: 'vm', label: 'VM', kind: 'ssh' })).toBe(c);
  });
});
