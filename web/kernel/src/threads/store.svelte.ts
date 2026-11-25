/**
 * Thread state for plugins and the kernel (06-client, Thread store; ADR 0014): one
 * subscription per open thread, events folded into a `ThreadView` once per frame. Threads
 * are keyed by their whole address, so the same ids on two cores never mix.
 */
import type {
  Attachment,
  Decision,
  EventNotification,
  HandoffMode,
  PermissionMode,
  QueueTrigger,
  Thread,
  UserInput,
} from '@kumikit/protocol';
import type { ThreadAddress, ThreadView } from '@kumikit/ui-sdk';

import type { CoreConnection } from '../connections/connection.svelte';
import { emptyView, foldEvents } from './fold';

/** Finds a connection by id. */
export type Connections = (id: string) => CoreConnection | undefined;

/** Schedules a fold; `requestAnimationFrame` in the page, immediate in tests. */
export type Schedule = (fn: () => void) => void;

const frame: Schedule =
  typeof requestAnimationFrame === 'function' ? (fn) => requestAnimationFrame(() => fn()) : (fn) => queueMicrotask(fn);

class OpenThread {
  view = $state.raw<ThreadView>(undefined as unknown as ThreadView);
  listeners = new Set<(v: ThreadView) => void>();
  #queue: EventNotification[] = [];
  #scheduled = false;
  #stop: () => void;

  constructor(
    readonly address: ThreadAddress,
    private readonly connection: CoreConnection | undefined,
    private readonly schedule: Schedule,
  ) {
    this.view = { ...emptyView(address), live: connection?.ready ?? false };
    this.#stop = connection?.subscribe(`thread:${address.thread}`, 0, (e) => this.#push(e)) ?? (() => {});
  }

  #push(e: EventNotification): void {
    this.#queue.push(e);
    if (this.#scheduled) return;
    this.#scheduled = true;
    this.schedule(() => this.flush());
  }

  /** Folds what has arrived. Runs once per frame; input never waits for it. */
  flush(): void {
    this.#scheduled = false;
    const batch = this.#queue;
    this.#queue = [];
    const thread =
      (this.connection?.threads[this.address.workspace] ?? []).find((t) => t.id === this.address.thread) ??
      this.connection?.sideThreads?.[this.address.thread];
    let next = foldEvents(this.view, batch);
    const live = this.connection?.ready ?? false;
    if (next.live !== live || next.thread !== thread) {
      next = { ...next, live };
      if (thread) next.thread = thread;
    }
    this.set(next);
  }

  set(view: ThreadView): void {
    if (view === this.view) return;
    this.view = view;
    for (const l of [...this.listeners]) l(view);
  }

  close(): void {
    this.#stop();
  }
}

export class ThreadStore {
  #open = new Map<string, OpenThread>();

  constructor(
    private readonly connections: Connections,
    private readonly schedule: Schedule = frame,
  ) {}

  #key(a: ThreadAddress): string {
    return `${a.connection}/${a.workspace}/${a.thread}`;
  }

  #connection(address: ThreadAddress): CoreConnection {
    const c = this.connections(address.connection);
    if (!c) throw new Error(`no core named ${address.connection}`);
    return c;
  }

  #get(address: ThreadAddress): OpenThread {
    const key = this.#key(address);
    let t = this.#open.get(key);
    if (!t) {
      t = new OpenThread(address, this.connections(address.connection), this.schedule);
      this.#open.set(key, t);
    }
    return t;
  }

  /** The current view (reactive in Svelte). */
  view(address: ThreadAddress): ThreadView {
    return this.#get(address).view;
  }

  /** Calls `fn` now and after every change; the thread closes when nobody watches. */
  watch(address: ThreadAddress, fn: (v: ThreadView) => void): () => void {
    const t = this.#get(address);
    t.listeners.add(fn);
    fn(t.view);
    return () => {
      t.listeners.delete(fn);
      if (t.listeners.size === 0) {
        t.close();
        this.#open.delete(this.#key(address));
      }
    };
  }

  /** Refolds the open threads of one connection, or all (after a status change). */
  refresh(connection?: string): void {
    for (const t of this.#open.values()) if (!connection || t.address.connection === connection) t.flush();
  }

  /** Sends a message (with optional attachments). One sent during a turn joins the
   * thread's queue, which the view follows from the log (`queue.*` events). */
  async send(address: ThreadAddress, text: string, attachments?: Attachment[]): Promise<void> {
    const input: UserInput = attachments && attachments.length > 0 ? { text, attachments } : { text };
    await this.#connection(address).call('thread.send', { thread: address.thread, input });
  }

  /** Sends a message to the running turn's agent. Returns `queued` when the turn cannot
   * be steered and the message went to the front of the queue instead. */
  async steer(address: ThreadAddress, text: string): Promise<'accepted' | 'queued'> {
    const r = await this.#connection(address).call('thread.steer', { thread: address.thread, input: { text } });
    return r.status;
  }

  /** Queues a message waiting on a trigger (ADR 0027). */
  async defer(
    address: ThreadAddress,
    text: string,
    trigger: QueueTrigger,
    attachments?: Attachment[],
  ): Promise<void> {
    const input: UserInput = attachments && attachments.length > 0 ? { text, attachments } : { text };
    await this.#connection(address).call('thread.defer', { thread: address.thread, input, trigger });
  }

  /** Replaces a queued message's text, keeping its attachments. */
  async queueUpdate(address: ThreadAddress, entry: string, text: string): Promise<void> {
    const queued = this.#get(address).view.queue.find((q) => q.id === entry);
    const input: UserInput = { ...(queued?.input ?? {}), text };
    await this.#connection(address).call('thread.queue.update', { thread: address.thread, entry, input });
  }

  /** Removes a queued message. */
  async queueRemove(address: ThreadAddress, entry: string): Promise<void> {
    await this.#connection(address).call('thread.queue.remove', { thread: address.thread, entry });
  }

  /** Moves a queued message before `before`, or to the end. */
  async queueMove(address: ThreadAddress, entry: string, before?: string): Promise<void> {
    const params: { thread: string; entry: string; before?: string } = { thread: address.thread, entry };
    if (before !== undefined) params.before = before;
    await this.#connection(address).call('thread.queue.move', params);
  }

  /** Starts a queued message now, stopping the running turn if there is one. */
  async queueSendNow(address: ThreadAddress, entry: string): Promise<void> {
    await this.#connection(address).call('thread.queue.sendNow', { thread: address.thread, entry });
  }

  /** Steers the running turn with a text-only queued message, removing its queue entry. */
  async queueSteer(address: ThreadAddress, entry: string): Promise<'accepted' | 'queued'> {
    const queued = this.#get(address).view.queue.find((q) => q.id === entry);
    if (!queued) return 'queued';
    const conn = this.#connection(address);
    const r = await conn.call('thread.steer', { thread: address.thread, input: { text: queued.input.text } });
    await conn.call('thread.queue.remove', { thread: address.thread, entry });
    return r.status;
  }

  /** Lets a held queue continue. */
  async queueResume(address: ThreadAddress): Promise<void> {
    await this.#connection(address).call('thread.queue.resume', { thread: address.thread });
  }

  /** Updates a thread's provider, model, permission mode, and/or title. */
  async update(
    address: ThreadAddress,
    patch: { provider?: string; model?: string; permissionMode?: PermissionMode; title?: string; handoff?: HandoffMode },
  ): Promise<Thread> {
    const conn = this.#connection(address);
    const params: {
      thread: string;
      provider?: string;
      model?: string;
      permissionMode?: PermissionMode;
      title?: string;
      handoff?: HandoffMode;
    } = {
      thread: address.thread,
    };
    if (patch.provider !== undefined) params.provider = patch.provider;
    if (patch.model !== undefined) params.model = patch.model;
    if (patch.permissionMode !== undefined) params.permissionMode = patch.permissionMode;
    if (patch.title !== undefined) params.title = patch.title;
    if (patch.handoff !== undefined) params.handoff = patch.handoff;
    const { thread } = await conn.call('thread.update', params);
    conn.noteThread(thread);
    this.refresh(address.connection);
    return thread;
  }

  /** Forks a thread up to `atTurn` or strictly before `beforeTurn`. */
  async fork(
    address: ThreadAddress,
    opts: { atTurn?: string; beforeTurn?: string; title?: string } = {},
  ): Promise<Thread> {
    const conn = this.#connection(address);
    const params: { thread: string; atTurn?: string; beforeTurn?: string; title?: string } = {
      thread: address.thread,
    };
    if (opts.atTurn !== undefined) params.atTurn = opts.atTurn;
    if (opts.beforeTurn !== undefined) params.beforeTurn = opts.beforeTurn;
    if (opts.title !== undefined) params.title = opts.title;
    const { thread } = await conn.call('thread.fork', params);
    conn.noteThread(thread);
    return thread;
  }

  /** Cancels the running turn; its queue is held, not dropped. */
  async cancel(address: ThreadAddress): Promise<void> {
    await this.#connection(address).call('thread.cancel', { thread: address.thread });
  }

  /** Answers an approval on the core the thread is on. */
  respond(connection: string, approval: string, decision: Decision): Promise<unknown> {
    const c = this.connections(connection);
    if (!c) return Promise.reject(new Error(`no core named ${connection}`));
    return c.call('thread.respondApproval', { approval, decision });
  }
}
