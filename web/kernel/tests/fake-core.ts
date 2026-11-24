/** An in-memory core for unit tests: answers methods from handlers, records requests,
 * and can push notifications or drop the connection. */
import type { EventNotification } from '@kumikit/protocol';

import { ConnectError, type Incoming, type LineConnection, type Platform, type ProfileInfo } from '../src/platform/index';
import type { Timers } from '../src/connections/connection.svelte';

type Handler = (params: Record<string, unknown>) => unknown;

export class FakeCore {
  handlers = new Map<string, Handler>();
  requests: { method: string; params: Record<string, unknown> }[] = [];
  /** Events per thread topic, replayed from `after` on subscribe. */
  logs = new Map<string, EventNotification[]>();
  connections = 0;
  refuse = false;
  /** When set, connecting fails with this code (as the shell reports it). */
  refuseWith: string | undefined;
  #on: ((m: Incoming) => void) | undefined;

  constructor() {
    this.handle('core.hello', (p) =>
      p.protocol === '0.8'
        ? { protocol: '0.8', core: { version: '0', name: 'fake', os: 'linux' }, plugins: [], capabilities: ['threads', 'approvals', 'plugins', 'providers', 'workspace_events'] }
        : { error: { code: 'protocol_mismatch', message: 'core speaks protocol 0.3' } },
    );
    this.handle('provider.list', () => ({ providers: [] }));
    this.handle('workspace.list', () => ({ workspaces: [] }));
    this.handle('thread.list', () => ({ threads: [] }));
    this.handle('events.unsubscribe', () => ({}));
    this.handle('events.subscribe', (p) => {
      const topic = String(p.topic);
      const after = Number(p.after);
      const log = this.logs.get(topic) ?? [];
      // Like the core: the result first, then the replay.
      queueMicrotask(() => {
        for (const e of log.filter((e) => e.seq > after)) this.notify('events.event', e);
      });
      return { head: log.at(-1)?.seq ?? 0 };
    });
  }

  handle(method: string, h: Handler): void {
    this.handlers.set(method, h);
  }

  /** Appends to a thread's log and delivers it live. */
  append(e: EventNotification): void {
    const log = this.logs.get(e.topic) ?? [];
    log.push(e);
    this.logs.set(e.topic, log);
    this.notify('events.event', e);
  }

  notify(method: string, params: unknown): void {
    this.#on?.({ type: 'line', line: JSON.stringify({ jsonrpc: '2.0', method, params }) });
  }

  /** The connection drops, as when the core stops. */
  drop(reason = 'gone'): void {
    const on = this.#on;
    this.#on = undefined;
    on?.({ type: 'closed', reason });
  }

  /** Connects a line client, as `Platform.connect` would. */
  async connect(on: (m: Incoming) => void): Promise<LineConnection> {
    if (this.refuseWith) throw new ConnectError(this.refuseWith, `refused: ${this.refuseWith}`);
    if (this.refuse) throw new Error('no core');
    this.connections++;
    this.#on = on;
    return {
      send: async (line) => {
        const msg = JSON.parse(line) as { id: number; method: string; params: Record<string, unknown> };
        this.requests.push({ method: msg.method, params: msg.params });
        const h = this.handlers.get(msg.method);
        const out = h ? h(msg.params) : { error: { code: 'method_not_found', message: msg.method } };
        const reply =
          typeof out === 'object' && out !== null && 'error' in out
            ? { jsonrpc: '2.0', id: msg.id, error: { code: -32000, message: (out as { error: { message: string } }).error.message, data: { code: (out as { error: { code: string } }).error.code } } }
            : { jsonrpc: '2.0', id: msg.id, result: out };
        queueMicrotask(() => on({ type: 'line', line: JSON.stringify(reply) }));
      },
      close: () => this.drop('closed by client'),
    };
  }

  platform(): Platform {
    return platformOf({ local: this });
  }
}

/** A platform whose profiles are these fake cores; the first is the home one. */
export function platformOf(cores: Record<string, FakeCore>, labels: Record<string, string> = {}, listed?: string[]): Platform {
  const names = listed ?? Object.keys(cores);
  const removed: string[] = [];
  return {
    kind: 'relay',
    home: Object.keys(cores)[0]!,
    connect: (profile, on) => {
      const core = cores[profile];
      if (!core || removed.includes(profile)) return Promise.reject(new ConnectError('unknown_profile', `no core named ${profile}`));
      return core.connect(on);
    },
    profiles: {
      editable: true,
      list: async () =>
        names
          .filter((n) => !removed.includes(n))
          .map((n, i): ProfileInfo => ({ name: n, label: labels[n] ?? n, kind: i === 0 ? 'local' : 'tls' })),
      probe: async () => 'SHA256:00',
      pair: async (r) => ({ name: r.name, label: r.label, kind: 'tls' }),
      addSsh: async (r) => ({ name: r.name, label: r.label, kind: 'ssh' }),
      remove: async (name) => {
        removed.push(name);
        return { revoked: true };
      },
    },
    pickBytes: async () => null,
    pickFile: async () => null,
      pickFolder: async () => null,
      pluginUrl: (id, path) => `/plugins/${id}/${path}`,
    clientPlugins: { list: async () => [], install: async () => { throw new Error('no'); }, remove: async () => {} },
    safeMode: false,
  };
}

/** Timers that run only when the test says so. */
export class ManualTimers implements Timers {
  pending: { fn: () => void; ms: number }[] = [];
  set(fn: () => void, ms: number): unknown {
    const t = { fn, ms };
    this.pending.push(t);
    return t;
  }
  clear(h: unknown): void {
    this.pending = this.pending.filter((t) => t !== h);
  }
  /** Runs the next timer; returns its delay. */
  fire(): number {
    const t = this.pending.shift();
    if (!t) throw new Error('no timer pending');
    t.fn();
    return t.ms;
  }
}

/** Lets queued microtasks and promise chains settle. */
export async function settle(): Promise<void> {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

export function event(thread: string, seq: number, kind: string, data: object): EventNotification {
  return { topic: `thread:${thread}`, seq, time: `2026-11-16T10:00:${String(seq).padStart(2, '0')}.000Z`, kind, data } as EventNotification;
}
