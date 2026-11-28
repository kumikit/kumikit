// @vitest-environment jsdom
import type { EventNotification, ProviderLimits } from '@kumikit/protocol';
import type { Connection, PluginContext, ThreadView } from '@kumikit/ui-sdk';
import { flushSync, mount, tick, unmount } from 'svelte';
import { expect, it } from 'vitest';

import Meter from '../src/Meter.svelte';
import plugin from '../src/index';
import { newer } from '../src/store.svelte';

const CLAUDE = 'acme.provider-claude/claude';
const PLAIN = 'acme.provider-plain/plain';

const limits = (pct: number): ProviderLimits => ({
  provider: CLAUDE,
  observed: '2026-11-28T12:01:00.000Z',
  limits: {
    plan: 'Pro',
    windows: [
      { id: 'five-hour', label: '5 hours', usedPercent: pct, resetsAt: '2026-11-28T14:59:00.000Z' },
      { id: 'weekly', label: 'Weekly', usedPercent: 19 },
    ],
  },
});

function fake() {
  const calls: { method: string; params: unknown }[] = [];
  let listener: ((e: EventNotification) => void) | undefined;
  let stored: ProviderLimits[] = [{ provider: CLAUDE }, { provider: PLAIN }];
  const summary = {
    totals: { inputTokens: 1500, outputTokens: 100, cachedInputTokens: 1200, cost: 12.5, turns: 3 },
    threads: 2,
    models: [{ provider: CLAUDE, model: 'claude-opus-5-5', inputTokens: 1500, outputTokens: 100, cachedInputTokens: 1200, cost: 12.5, turns: 3 }],
    daily: [{ date: '2026-11-28', tokens: 1600, cost: 12.5 }],
  };
  const connection = {
    id: 'local',
    ready: true,
    async call(method: string, params: unknown) {
      calls.push({ method, params });
      switch (method) {
        case 'provider.list':
          return {
            providers: [
              { id: CLAUDE, name: 'Claude Code', plugin: 'acme.provider-claude', readsLimits: true },
              { id: PLAIN, name: 'Plain', plugin: 'acme.provider-plain' },
            ],
          };
        case 'usage.limits':
          return { providers: stored };
        case 'usage.summary':
          return summary;
        case 'usage.refresh':
          stored = [limits(55), { provider: PLAIN }];
          return limits(55);
        default:
          throw new Error(`unexpected ${method}`);
      }
    },
    subscribe(topic: string, _after: number, fn: (e: EventNotification) => void) {
      expect(topic).toBe('usage');
      listener = fn;
      return () => (listener = undefined);
    },
    models: async () => [],
  } as unknown as Connection;
  const view: ThreadView = {
    address: { connection: 'local', workspace: 'w', thread: 't' },
    thread: { id: 't', workspace: 'w', provider: CLAUDE, model: 'claude-opus-5-5', status: 'idle', created: '', updated: '' },
    status: 'idle',
    seq: 0,
    turns: [],
    pendingApprovals: [],
    queued: [],
    queue: [],
    live: true,
  } as unknown as ThreadView;
  const notes: string[] = [];
  const ctx = {
    threads: {
      watch(_a: unknown, fn: (v: ThreadView) => void) {
        fn(view);
        return () => {};
      },
    },
    notify: (m: string) => notes.push(m),
  } as unknown as PluginContext;
  // Like the core: a pushed snapshot is also what `usage.limits` returns afterwards.
  const emit = (e: EventNotification) => {
    if (e.kind === 'usage.limits') stored = stored.map((p) => (p.provider === e.data.provider ? e.data : p));
    listener?.(e);
  };
  return { connection, ctx, calls, notes, emit, subscribed: () => listener !== undefined };
}

async function settle() {
  for (let i = 0; i < 5; i++) {
    await tick();
    await new Promise((r) => setTimeout(r, 0));
  }
  flushSync();
}

it('keeps a pushed snapshot that is newer than a read', () => {
  const old: ProviderLimits = { provider: CLAUDE, observed: '2026-11-28T12:00:00.000Z' };
  const fresh: ProviderLimits = { provider: CLAUDE, observed: '2026-11-28T12:05:00.000Z' };
  expect(newer(old, fresh)).toBe(fresh);
  expect(newer(fresh, old)).toBe(fresh);
  expect(newer(old, undefined)).toBe(old);
  expect(newer(old, { provider: CLAUDE })).toBe(old);
});

it('shows the fullest window of the thread’s provider and opens Agent usage on it', async () => {
  const f = fake();
  const target = document.createElement('div');
  document.body.append(target);
  const c = mount(Meter, { target, props: { ctx: f.ctx, address: { connection: 'local', workspace: 'w', thread: 't' }, connection: f.connection } });
  await settle();
  // Nothing reported yet: no meter.
  expect(target.querySelector('.kk-usage-meter')).toBeNull();
  expect(f.subscribed()).toBe(true);

  f.emit({ topic: 'usage', seq: 1, time: '', kind: 'usage.limits', data: limits(86) } as EventNotification);
  flushSync();
  const meter = target.querySelector<HTMLButtonElement>('.kk-usage-meter')!;
  expect(meter.textContent?.replace(/\s+/g, ' ').trim()).toBe('5h 86%');
  expect(meter.dataset.level).toBe('warn');
  expect(meter.getAttribute('aria-label')).toContain('5 hours limit 86% used');
  expect(meter.title).toContain('Weekly: 19% used');

  meter.click();
  await settle();
  const dialog = target.querySelector('dialog')!;
  expect(dialog).not.toBeNull();
  const name = (t: Element) => t.querySelector('span:not([aria-hidden])')?.textContent?.trim();
  const tabs = [...dialog.querySelectorAll('[role=tab]')].map(name);
  expect(tabs).toEqual(['Overview', 'Claude Code', 'Plain']);
  expect(name(dialog.querySelector('[role=tab][aria-selected=true]')!)).toBe('Claude Code');
  const meters = [...dialog.querySelectorAll('[role=meter]')].map((m) => m.getAttribute('aria-valuenow'));
  expect(meters).toEqual(['86', '19']);
  expect(dialog.textContent).toContain('Pro');
  expect(dialog.textContent).toContain('$12.50');
  expect(dialog.textContent).toContain('claude-opus-5-5');
  expect(f.calls.some((x) => x.method === 'usage.summary' && (x.params as { provider?: string }).provider === CLAUDE)).toBe(true);

  // Refresh asks the provider and shows the new reading.
  const refresh = dialog.querySelector<HTMLButtonElement>('button[aria-label^="Refresh Claude"]')!;
  expect(refresh.disabled).toBe(false);
  refresh.click();
  await settle();
  expect(f.calls.filter((x) => x.method === 'usage.refresh')).toEqual([{ method: 'usage.refresh', params: { provider: CLAUDE } }]);
  expect(target.querySelector('.kk-usage-meter')?.textContent?.replace(/\s+/g, ' ').trim()).toBe('5h 55%');

  // A provider without a reader cannot refresh.
  dialog.querySelector<HTMLButtonElement>('[data-tab="' + PLAIN + '"]')!.click();
  await settle();
  expect(dialog.querySelector<HTMLButtonElement>('button[aria-label^="Refresh Plain"]')!.disabled).toBe(true);
  expect(dialog.textContent).toContain("hasn't reported plan limits");

  unmount(c);
  expect(f.subscribed()).toBe(false);
  target.remove();
});

it('opens Agent usage from the palette command even without a meter', async () => {
  const f = fake();
  let handler: ((args?: unknown) => unknown) | undefined;
  const ctx = {
    ...f.ctx,
    connection: f.connection,
    slots: { register: () => () => {} },
    commands: {
      register(spec: { id: string }, fn: (args?: unknown) => unknown) {
        if (spec.id === 'usage.open') handler = fn;
        return () => {};
      },
      run: async () => {},
    },
  } as unknown as PluginContext;

  await plugin.activate(ctx);
  expect(handler).toBeDefined();
  handler!();
  await settle();
  const dialog = document.body.querySelector('dialog.kk-usage');
  expect(dialog).not.toBeNull();
  expect(dialog!.querySelector('[role=tab][aria-selected=true]')?.textContent).toContain('Overview');
  dialog!.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!.click();
  await settle();
  expect(document.body.querySelector('dialog.kk-usage')).toBeNull();
  expect(f.subscribed()).toBe(false);
  await plugin.deactivate?.();
});
