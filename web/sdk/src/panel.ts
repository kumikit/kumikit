/**
 * The UI SDK for sandboxed panels (07-ui-plugins, Level 2; ADR 0019): the level-3 API made
 * asynchronous, over the `MessagePort` the UI kernel hands the panel when it loads.
 *
 * ```ts
 * import { connectPanel } from '@kumikit/ui-sdk/panel';
 * const ctx = await connectPanel();
 * const { plugins } = await ctx.connection.call('plugin.list', {});
 * ```
 *
 * The bridge (version 1): the UI kernel posts `{ type: 'kumikit.panel', version: 1 }` with
 * a port to the panel's window once; then, on the port, it sends `hello`, answers requests
 * `{ id, method, params }` with `{ id, result }` or `{ id, error: { code, message } }`, and
 * sends notifications (`theme`, `event`, `thread`). Calls are limited by the UI kernel to
 * the tab's core and to what the panel may read; others fail with `permission_denied`.
 */
import type { EventNotification } from '@kumikit/protocol';

import type { Dispose, ThreadAddress, ThreadView } from './index';

/** The bridge version this SDK speaks. */
export const BRIDGE_VERSION = 1;

/** A refused or failed call: `code` is a protocol error code (`permission_denied`, …). */
export class PanelError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

/** Where the panel is shown. */
export interface PanelHello {
  /** The slot (`inspector.tab`, `sidebar.section`, `view`, `settings.page`). */
  slot: string;
  /** The contribution's id from the manifest. */
  contribution: string;
  /** The tab's core and workspace (and thread), when there is a tab. */
  address?: { connection: string; workspace?: string; thread?: string };
  theme: { tokens: Record<string, string>; appearance: 'light' | 'dark' };
}

export interface PanelContext {
  readonly slot: string;
  readonly contribution: string;
  readonly address: PanelHello['address'];
  /** The tab's core, limited to what panels may call. */
  connection: {
    call(method: string, params: unknown): Promise<unknown>;
    subscribe(topic: string, after: number, fn: (event: EventNotification) => void): Promise<Dispose>;
  };
  threads: {
    watch(address: ThreadAddress, fn: (view: ThreadView) => void): Promise<Dispose>;
  };
  theme: {
    /** The current tokens; also applied to this document as `--kk-*` properties. */
    readonly tokens: Readonly<Record<string, string>>;
    readonly appearance: 'light' | 'dark';
    onChange(fn: () => void): Dispose;
  };
  storage: {
    get(key: string): Promise<string | undefined>;
    set(key: string, value: string): Promise<void>;
    delete(key: string): Promise<void>;
  };
  notify(message: string, tone?: 'info' | 'success' | 'warning' | 'danger'): Promise<void>;
  commands: {
    /** Runs one of this plugin's own commands. */
    run(id: string, args?: unknown): Promise<unknown>;
  };
}

/** A token name as a CSS custom property: `surface.panel` → `--kk-surface-panel`. */
export const tokenProperty = (name: string): string => `--kk-${name.replace(/\./g, '-')}`;

/** Applies tokens to a document through one constructed stylesheet (no inline styles, so
 * the panel's CSP needs none). */
export function applyTokens(doc: Document, tokens: Readonly<Record<string, string>>): void {
  const body = Object.entries(tokens)
    .filter(([name, value]) => /^[a-z0-9.-]+$/.test(name) && !/[;{}<>\\]/.test(value))
    .map(([name, value]) => `${tokenProperty(name)}: ${value};`)
    .join(' ');
  const sheet = new CSSStyleSheet();
  sheet.replaceSync(`:root { ${body} }`);
  doc.adoptedStyleSheets = [sheet];
}

type Message =
  | { type: 'hello'; hello: PanelHello }
  | { type: 'theme'; tokens: Record<string, string>; appearance: 'light' | 'dark' }
  | { type: 'event'; sub: number; event: EventNotification }
  | { type: 'thread'; sub: number; view: ThreadView }
  | { id: number; result?: unknown; error?: { code: string; message: string } };

/** Waits for the UI kernel's port and returns the panel's context. */
export function connectPanel(win: Window = window, timeoutMs = 10_000): Promise<PanelContext> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new PanelError('internal', 'the UI kernel did not connect this panel')), timeoutMs);
    const onWindow = (e: MessageEvent) => {
      const d = e.data as { type?: unknown; version?: unknown } | null;
      if (!d || d.type !== 'kumikit.panel' || e.source !== win.parent || !e.ports[0]) return;
      win.removeEventListener('message', onWindow);
      if (d.version !== BRIDGE_VERSION) {
        clearTimeout(timer);
        reject(new PanelError('protocol_mismatch', `the UI kernel speaks bridge ${String(d.version)}`));
        return;
      }
      start(e.ports[0], win, (ctx) => {
        clearTimeout(timer);
        resolve(ctx);
      });
    };
    win.addEventListener('message', onWindow);
  });
}

function start(port: MessagePort, win: Window, ready: (ctx: PanelContext) => void): void {
  let next = 0;
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: unknown) => void }>();
  const events = new Map<number, (e: EventNotification) => void>();
  const views = new Map<number, (v: ThreadView) => void>();
  const themeListeners = new Set<() => void>();
  let theme: PanelHello['theme'] = { tokens: {}, appearance: 'light' };
  const request = (method: string, params: unknown): Promise<unknown> =>
    new Promise((resolve, reject) => {
      const id = ++next;
      pending.set(id, { resolve, reject });
      port.postMessage({ id, method, params });
    });

  port.onmessage = (e: MessageEvent<Message>) => {
    const m = e.data;
    if ('id' in m) {
      const p = pending.get(m.id);
      pending.delete(m.id);
      if (m.error) p?.reject(new PanelError(m.error.code, m.error.message));
      else p?.resolve(m.result);
      return;
    }
    switch (m.type) {
      case 'hello':
        theme = m.hello.theme;
        applyTokens(win.document, theme.tokens);
        ready(context(m.hello));
        break;
      case 'theme':
        theme = { tokens: m.tokens, appearance: m.appearance };
        applyTokens(win.document, theme.tokens);
        for (const fn of themeListeners) fn();
        break;
      case 'event':
        events.get(m.sub)?.(m.event);
        break;
      case 'thread':
        views.get(m.sub)?.(m.view);
        break;
    }
  };

  const context = (hello: PanelHello): PanelContext => ({
    slot: hello.slot,
    contribution: hello.contribution,
    address: hello.address,
    connection: {
      call: (method, params) => request('call', { method, params }),
      async subscribe(topic, after, fn) {
        const { sub } = (await request('subscribe', { topic, after })) as { sub: number };
        events.set(sub, fn);
        return () => {
          events.delete(sub);
          void request('unsubscribe', { sub });
        };
      },
    },
    threads: {
      async watch(address, fn) {
        const { sub } = (await request('watch', { address })) as { sub: number };
        views.set(sub, fn);
        return () => {
          views.delete(sub);
          void request('unwatch', { sub });
        };
      },
    },
    theme: {
      get tokens() {
        return theme.tokens;
      },
      get appearance() {
        return theme.appearance;
      },
      onChange(fn) {
        themeListeners.add(fn);
        return () => themeListeners.delete(fn);
      },
    },
    storage: {
      get: async (key) => (await request('storage.get', { key })) as string | undefined,
      set: async (key, value) => void (await request('storage.set', { key, value })),
      delete: async (key) => void (await request('storage.delete', { key })),
    },
    notify: async (message, tone) => void (await request('notify', { message, tone })),
    commands: {
      run: (id, args) => request('commands.run', { id, args }),
    },
  });
  port.start();
}
