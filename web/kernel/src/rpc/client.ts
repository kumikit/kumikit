/**
 * A JSON-RPC 2.0 client over one line-based connection (`docs/spec/02-protocol.md`).
 * Transport-free: it takes a `send` function and is fed incoming lines, so tests drive
 * it in memory.
 */
import type { ErrorCode, Methods, Notifications } from '@kumikit/protocol';

/** An error answer from the core, or a failure of the connection. */
export class RpcError extends Error {
  /** The stable `data.code` (02-protocol, Errors); unknown codes read as `internal`. */
  readonly code: ErrorCode | 'disconnected';

  constructor(code: ErrorCode | 'disconnected', message: string) {
    super(message);
    this.code = code;
    this.name = 'RpcError';
  }
}

const KNOWN: ReadonlySet<string> = new Set<ErrorCode>([
  'parse_error',
  'invalid_request',
  'method_not_found',
  'invalid_params',
  'not_found',
  'permission_denied',
  'unsupported',
  'conflict',
  'handshake_required',
  'protocol_mismatch',
  'plugin_failed',
  'internal',
]);

type Pending = { resolve: (v: unknown) => void; reject: (e: RpcError) => void };

/** A notification from the core. */
export type NotificationMessage = {
  [N in keyof Notifications]: { method: N; params: Notifications[N] };
}[keyof Notifications];

export class RpcClient {
  #next = 0;
  #pending = new Map<number, Pending>();
  #closed: RpcError | undefined;

  constructor(
    private readonly send: (line: string) => Promise<void>,
    private readonly onNotification: (n: NotificationMessage) => void,
  ) {}

  /** Calls a method. Rejects with an [`RpcError`] on an error answer or a lost
   * connection. */
  call<M extends keyof Methods>(
    method: M,
    params: Methods[M]['params'],
  ): Promise<Methods[M]['result']> {
    if (this.#closed) return Promise.reject(this.#closed);
    const id = ++this.#next;
    const line = JSON.stringify({ jsonrpc: '2.0', id, method, params });
    return new Promise((resolve, reject) => {
      this.#pending.set(id, { resolve: resolve as (v: unknown) => void, reject });
      this.send(line).catch((e: unknown) => {
        this.#pending.delete(id);
        reject(new RpcError('disconnected', String(e)));
      });
    });
  }

  /** Handles one line from the core. Lines that are not JSON-RPC are ignored. */
  receive(line: string): void {
    let msg: unknown;
    try {
      msg = JSON.parse(line);
    } catch {
      return;
    }
    if (typeof msg !== 'object' || msg === null) return;
    const m = msg as Record<string, unknown>;
    if (typeof m.method === 'string' && m.id === undefined) {
      this.onNotification({ method: m.method, params: m.params } as NotificationMessage);
      return;
    }
    if (typeof m.id !== 'number') return;
    const pending = this.#pending.get(m.id);
    if (!pending) return;
    this.#pending.delete(m.id);
    if ('error' in m && typeof m.error === 'object' && m.error !== null) {
      const e = m.error as { message?: unknown; data?: { code?: unknown } };
      const code = typeof e.data?.code === 'string' && KNOWN.has(e.data.code) ? e.data.code : 'internal';
      pending.reject(new RpcError(code as ErrorCode, String(e.message ?? 'error')));
    } else {
      pending.resolve(m.result);
    }
  }

  /** Fails every pending call and any later one: the connection is gone. */
  close(reason: string): void {
    this.#closed = new RpcError('disconnected', reason);
    for (const p of this.#pending.values()) p.reject(this.#closed);
    this.#pending.clear();
  }

  /** Calls waiting for an answer. */
  get pending(): number {
    return this.#pending.size;
  }
}
