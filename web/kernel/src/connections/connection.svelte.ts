/**
 * One connection to a core (06-client, Connection manager and State per connection):
 * handshake, reconnect with backoff, and the one wire subscription per topic, fanned out
 * to every listener.
 */
import {
  PROTOCOL_VERSION,
  type EventNotification,
  type HelloResult,
  type Methods,
  type ModelInfo,
  type PluginInfo,
  type ProviderInfo,
  type Thread,
  type Workspace,
} from '@kumikit/protocol';

import { ConnectError, type LineConnection, type Platform, type ProfileInfo } from '../platform/index';
import { type NotificationMessage, RpcClient, RpcError } from '../rpc/client';

/**
 * `unauthorized` (this device was revoked, or the core forgot it) and
 * `certificate_changed` stop retrying: only the user can fix them.
 */
export type ConnectionStatus = 'connecting' | 'ready' | 'reconnecting' | 'mismatch' | 'unauthorized' | 'certificate_changed';

/** Statuses after which the connection does not retry by itself. */
const STOPPED: ConnectionStatus[] = ['mismatch', 'unauthorized', 'certificate_changed'];

/** Timers, injected so tests control time. */
export interface Timers {
  set(fn: () => void, ms: number): unknown;
  clear(handle: unknown): void;
}

const realTimers: Timers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

/** Backoff between reconnect attempts: 0.5 s doubling to 30 s. */
export function backoff(attempt: number): number {
  return Math.min(500 * 2 ** attempt, 30_000);
}

type Listener = { fn: (e: EventNotification) => void; cursor: number };

interface Subscription {
  listeners: Set<Listener>;
  /** Thread topics replay from a cursor; workspace topics do not. */
  logged: boolean;
}

export interface ConnectionOptions {
  timers?: Timers;
  client?: { name: string; version: string };
  /** The profile's label and kind; `local` by default. */
  profile?: Omit<ProfileInfo, 'name'>;
}

export class CoreConnection {
  readonly id: string;
  /** What the user calls this core. */
  readonly label: string;
  readonly kind: ProfileInfo['kind'];
  status = $state<ConnectionStatus>('connecting');
  /** Why the last attempt failed, in plain words. */
  error = $state<string | undefined>();
  hello = $state.raw<HelloResult | undefined>();
  plugins = $state.raw<PluginInfo[]>([]);
  providers = $state.raw<ProviderInfo[]>([]);
  workspaces = $state.raw<Workspace[]>([]);
  /** Threads per workspace id, most recently updated first. */
  threads = $state.raw<Record<string, Thread[]>>({});
  /** The version a mismatched core speaks, if it said. */
  coreProtocol = $state<string | undefined>();

  #platform: Platform;
  #timers: Timers;
  #client: { name: string; version: string };
  #line: LineConnection | undefined;
  #rpc: RpcClient | undefined;
  #subs = new Map<string, Subscription>();
  #attempt = 0;
  #retry: unknown;
  #stopped = false;
  /** A connect attempt is in flight. */
  #connecting = false;
  #models = new Map<string, Promise<ModelInfo[]>>();
  #watched = new Set<string>();

  constructor(id: string, platform: Platform, options: ConnectionOptions = {}) {
    this.id = id;
    this.label = options.profile?.label ?? 'This computer';
    this.kind = options.profile?.kind ?? 'local';
    this.#platform = platform;
    this.#timers = options.timers ?? realTimers;
    this.#client = options.client ?? { name: 'kumikit-desktop', version: '0.1.0' };
  }

  get ready(): boolean {
    return this.status === 'ready';
  }

  /** Connects; on failure keeps retrying with backoff until `stop`. */
  async start(): Promise<void> {
    this.#stopped = false;
    await this.#connect();
  }

  /** Tries again now instead of waiting out the backoff (after the network is back). */
  reconnectNow(): void {
    if (this.#stopped || this.status === 'ready' || this.#rpc || this.#connecting) return;
    this.#timers.clear(this.#retry);
    this.#attempt = 0;
    void this.#connect();
  }

  stop(): void {
    this.#stopped = true;
    this.#timers.clear(this.#retry);
    this.#rpc?.close('stopped');
    this.#line?.close();
  }

  /** Calls a method on the core. Rejects with `disconnected` while not ready. */
  call<M extends keyof Methods>(method: M, params: Methods[M]['params']): Promise<Methods[M]['result']> {
    if (!this.#rpc || this.status !== 'ready') {
      return Promise.reject(new RpcError('disconnected', 'not connected'));
    }
    return this.#rpc.call(method, params);
  }

  /** The models of a provider; asked once per connection. */
  models(provider: string): Promise<ModelInfo[]> {
    let p = this.#models.get(provider);
    if (!p) {
      p = this.call('provider.models', { provider }).then((r) => r.models);
      p.catch(() => this.#models.delete(provider));
      this.#models.set(provider, p);
    }
    return p;
  }

  /** Follows a topic. Several listeners share one wire subscription; on a thread topic
   * each gets only events after its own cursor, also across reconnects. */
  subscribe(topic: string, after: number, fn: (e: EventNotification) => void): () => void {
    const logged = topic.startsWith('thread:');
    let sub = this.#subs.get(topic);
    const listener: Listener = { fn, cursor: after };
    const fresh = !sub;
    if (!sub) {
      sub = { listeners: new Set(), logged };
      this.#subs.set(topic, sub);
    }
    const lowest = Math.min(...[...sub.listeners].map((l) => l.cursor), Infinity);
    sub.listeners.add(listener);
    // A new listener that needs older events replays the topic from its cursor; the
    // others skip what they already have.
    if (this.ready && (fresh || (logged && after < lowest))) void this.#wire(topic, logged ? after : 0);
    return () => {
      sub.listeners.delete(listener);
      if (sub.listeners.size === 0) {
        this.#subs.delete(topic);
        if (this.ready) void this.call('events.unsubscribe', { topic }).catch(() => {});
      }
    };
  }

  /** Keeps `threads[workspace]` current through the workspace topic. */
  watchWorkspace(workspace: string): () => void {
    this.#watched.add(workspace);
    const stop = this.subscribe(`workspace:${workspace}`, 0, (e) => {
      if (e.kind === 'thread.created' || e.kind === 'thread.updated' || e.kind === 'thread.archived') {
        this.#mergeThread(e.data.thread);
      } else if (e.kind === 'thread.deleted') {
        this.removeThread(workspace, e.data.thread.id);
      }
    });
    if (this.ready) void this.#listThreads(workspace);
    return () => {
      this.#watched.delete(workspace);
      stop();
    };
  }

  /** Records a thread this client created or changed, before the topic reports it. */
  noteThread(thread: Thread): void {
    this.#mergeThread(thread);
  }

  /** Removes a deleted thread from the workspace's thread list immediately. */
  removeThread(workspace: string, threadId: string): void {
    const list = this.threads[workspace];
    if (!list) return;
    const next = list.filter((t) => t.id !== threadId);
    if (next.length !== list.length) {
      this.threads = { ...this.threads, [workspace]: next };
    }
  }

  /** Records a workspace created or opened outside `openWorkspace` (such as a Git worktree). */
  noteWorkspace(workspace: Workspace): void {
    if (!this.workspaces.some((w) => w.id === workspace.id)) {
      this.workspaces = [...this.workspaces, workspace];
    }
  }

  /** Removes a deleted workspace and its cached threads immediately. */
  removeWorkspace(workspaceId: string): void {
    this.#watched.delete(workspaceId);
    if (this.workspaces.some((w) => w.id === workspaceId)) {
      this.workspaces = this.workspaces.filter((w) => w.id !== workspaceId);
    }
    if (workspaceId in this.threads) {
      const next = { ...this.threads };
      delete next[workspaceId];
      this.threads = next;
    }
  }

  /** Opens a workspace on the core and adds it to the list. */
  async openWorkspace(path: string): Promise<Workspace> {
    const { workspace } = await this.call('workspace.open', { path });
    this.noteWorkspace(workspace);
    return workspace;
  }

  /** For a core of another version that knows `core.shutdown` (0.3 and later): stops
   * it, so the next connection starts the bundled core. */
  async restartMismatchedCore(): Promise<boolean> {
    const version = this.coreProtocol;
    if (!version || !this.#rpc) return false;
    try {
      await this.#rpc.call('core.hello', { protocol: version, client: this.#client });
      await this.#rpc.call('core.shutdown', {});
    } catch {
      return false;
    }
    this.status = 'reconnecting';
    return true;
  }

  // ---- internals ----

  async #connect(): Promise<void> {
    this.#connecting = true;
    try {
      await this.#attemptConnect();
    } finally {
      this.#connecting = false;
    }
  }

  async #attemptConnect(): Promise<void> {
    let line: LineConnection;
    let rpc: RpcClient | undefined;
    try {
      line = await this.#platform.connect(this.id, (m) => {
        if (m.type === 'line') rpc?.receive(m.line);
        else this.#lost(m.reason, rpc);
      });
    } catch (e) {
      if (e instanceof ConnectError && (e.code === 'unauthorized' || e.code === 'certificate_changed')) {
        this.error = e.message;
        this.status = e.code;
        return;
      }
      this.#scheduleRetry(String(e instanceof Error ? e.message : e));
      return;
    }
    rpc = new RpcClient(
      (l) => line.send(l),
      (n) => this.#notify(n),
    );
    this.#line = line;
    this.#rpc = rpc;
    let hello: HelloResult;
    try {
      hello = await rpc.call('core.hello', { protocol: PROTOCOL_VERSION, client: this.#client });
    } catch (e) {
      if (e instanceof RpcError && e.code === 'protocol_mismatch') {
        this.coreProtocol = /protocol (\d+\.\d+)/.exec(e.message)?.[1];
        this.error = e.message;
        this.status = 'mismatch';
        return;
      }
      line.close();
      this.#scheduleRetry(e instanceof Error ? e.message : String(e));
      return;
    }
    this.#attempt = 0;
    this.error = undefined;
    this.hello = hello;
    this.plugins = hello.plugins;
    this.status = 'ready';
    this.#models.clear();
    await Promise.allSettled([this.#refreshProviders(), this.#refreshWorkspaces()]);
    for (const [topic, sub] of this.#subs) {
      const after = sub.logged ? Math.min(...[...sub.listeners].map((l) => l.cursor)) : 0;
      void this.#wire(topic, after);
    }
    for (const ws of this.#watched) void this.#listThreads(ws);
  }

  #lost(reason: string, rpc: RpcClient | undefined): void {
    rpc?.close(reason);
    if (this.#rpc !== rpc) return;
    this.#rpc = undefined;
    this.#line = undefined;
    if (this.#stopped || STOPPED.includes(this.status)) return;
    this.#scheduleRetry(reason);
  }

  #scheduleRetry(reason: string): void {
    if (this.#stopped) return;
    this.error = reason;
    this.status = this.hello ? 'reconnecting' : 'connecting';
    const wait = backoff(this.#attempt++);
    this.#retry = this.#timers.set(() => void this.#connect(), wait);
  }

  async #wire(topic: string, after: number): Promise<void> {
    try {
      await this.call('events.subscribe', { topic, after });
    } catch {
      // The next reconnect resubscribes.
    }
  }

  #notify(n: NotificationMessage): void {
    if (n.method === 'core.pluginsChanged') {
      this.plugins = n.params.plugins;
      this.#models.clear();
      void this.#refreshProviders();
      return;
    }
    if (n.method === 'events.event') {
      const e = n.params;
      const sub = this.#subs.get(e.topic);
      if (!sub) return;
      for (const l of [...sub.listeners]) {
        if (sub.logged) {
          if (e.seq <= l.cursor) continue;
          l.cursor = e.seq;
        }
        l.fn(e);
      }
    }
  }

  async #refreshProviders(): Promise<void> {
    if (!this.hello?.capabilities.includes('providers')) return;
    this.providers = (await this.call('provider.list', {})).providers;
  }

  async #refreshWorkspaces(): Promise<void> {
    this.workspaces = (await this.call('workspace.list', {})).workspaces;
  }

  async #listThreads(workspace: string): Promise<void> {
    try {
      const { threads } = await this.call('thread.list', { workspace, includeArchived: true });
      for (const t of threads) this.#mergeThread(t);
    } catch {
      // Shown again after the next reconnect.
    }
  }

  /** Keeps the newer copy of a thread (by `updated`), so a list and live events can
   * arrive in either order (02-protocol, Workspace topic). */
  #mergeThread(thread: Thread): void {
    const list = this.threads[thread.workspace] ?? [];
    const old = list.find((t) => t.id === thread.id);
    if (old && old.updated > thread.updated) return;
    const next = [thread, ...list.filter((t) => t.id !== thread.id)].sort((a, b) =>
      a.updated < b.updated ? 1 : a.updated > b.updated ? -1 : 0,
    );
    this.threads = { ...this.threads, [thread.workspace]: next };
  }
}
