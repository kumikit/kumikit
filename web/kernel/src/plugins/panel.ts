/**
 * Sandboxed panels (07-ui-plugins, Level 2; ADR 0019): the frame, the bridge and what a
 * panel may call. The SDK side is `@kumikit/ui-sdk/panel`.
 */
import type { Slot } from '@kumikit/ui-sdk';

import type { Kernel } from '../app.svelte';

/** Slots a panel may contribute to. */
export const PANEL_SLOTS: readonly Slot[] = ['sidebar.section', 'inspector.tab', 'view', 'settings.page'];

/** Largest bridge message, and most calls in flight per panel. */
export const MAX_MESSAGE = 1 << 20;
export const MAX_IN_FLIGHT = 64;

/** Kernel methods a panel may call: reads only. */
const READS = new Set(['workspace.list', 'thread.list', 'thread.get', 'provider.list', 'provider.models', 'plugin.list']);

/** What the policy needs to know about the panel's place. */
export interface PanelScope {
  /** The tab's workspace, if there is a tab. */
  workspace?: string;
  /** The workspace a thread is in, if the UI kernel knows it. */
  threadWorkspace(thread: string): string | undefined;
  /** Plugin ids from the plugin's `requires.core`. */
  requires: readonly string[];
}

/** A panel's params for a `<pluginId>/*` call: a `workspace` it names is replaced by the
 * tab's (or removed without one), so a panel reaches no other workspace through a plugin. */
export function panelMethodParams(params: unknown, workspace: string | undefined): Record<string, unknown> {
  const p = { ...(typeof params === 'object' && params !== null && !Array.isArray(params) ? (params as Record<string, unknown>) : {}) };
  delete p.workspace;
  if (workspace !== undefined) p.workspace = workspace;
  return p;
}

/** Whether a panel may make this core call: `null` if it may, otherwise why not. */
export function panelRefusal(method: string, params: unknown, scope: PanelScope): string | null {
  const p = (typeof params === 'object' && params !== null ? params : {}) as Record<string, unknown>;
  const slash = method.indexOf('/');
  if (slash > 0) {
    return scope.requires.includes(method.slice(0, slash)) ? null : `${method}: the panel does not require ${method.slice(0, slash)}`;
  }
  if (!READS.has(method)) return `${method}: panels may only read`;
  const inTab = (thread: unknown) => typeof thread === 'string' && scope.workspace !== undefined && scope.threadWorkspace(thread) === scope.workspace;
  if (method === 'thread.list' && (scope.workspace === undefined || p.workspace !== scope.workspace)) {
    return 'thread.list: only the tab’s workspace';
  }
  if (method === 'thread.get' && !inTab(p.thread)) return 'thread.get: only threads of the tab’s workspace';
  return null;
}

/** Whether a panel may follow this topic. */
export function topicRefusal(topic: string, scope: PanelScope): string | null {
  if (topic.startsWith('workspace:')) {
    return scope.workspace !== undefined && topic === `workspace:${scope.workspace}` ? null : 'only the tab’s workspace';
  }
  if (topic.startsWith('thread:')) {
    const thread = topic.slice('thread:'.length);
    return scope.workspace !== undefined && scope.threadWorkspace(thread) === scope.workspace ? null : 'only threads of the tab’s workspace';
  }
  return `${topic}: not a topic panels may follow`;
}

interface Request {
  id: number;
  method: string;
  params?: Record<string, unknown>;
}

/** What a panel plugin's manifest says. */
export interface PanelPlugin {
  id: string;
  entry: string;
  requires: string[];
}

/**
 * Mounts a panel into `el`: a sandboxed frame of the plugin's entry, and the bridge.
 * Returns the unmount.
 */
export function mountPanel(
  k: Kernel,
  plugin: PanelPlugin,
  slot: string,
  contribution: { id: string; title: string },
  el: HTMLElement,
): () => void {
  const tab = k.activeTab;
  // The panel acts for the active tab at the time of each call (a sidebar or view panel
  // is not tied to one tab).
  const here = () => {
    const t = k.activeTab;
    return { connectionId: t?.connection ?? k.connection.id, workspace: t?.workspace };
  };
  const scope: PanelScope = {
    get workspace() {
      return here().workspace;
    },
    threadWorkspace(thread) {
      const conn = k.connectionFor(here().connectionId);
      if (!conn) return undefined;
      for (const [ws, threads] of Object.entries(conn.threads)) if (threads.some((t) => t.id === thread)) return ws;
      return undefined;
    },
    requires: plugin.requires,
  };
  const frame = el.ownerDocument.createElement('iframe');
  frame.setAttribute('sandbox', 'allow-scripts');
  frame.setAttribute('referrerpolicy', 'no-referrer');
  frame.setAttribute('allow', '');
  frame.title = contribution.title;
  frame.className = 'kk-panel';
  const cleanups: (() => void)[] = [];
  const subs = new Map<number, () => void>();
  let port: MessagePort | undefined;
  let loads = 0;
  let inFlight = 0;
  let nextSub = 0;
  let closed = false;

  const close = (why?: string) => {
    if (closed) return;
    closed = true;
    for (const d of subs.values()) d();
    subs.clear();
    for (const c of cleanups) c();
    port?.close();
    frame.remove();
    if (why) {
      const p = el.ownerDocument.createElement('p');
      p.className = 'kk-muted';
      p.textContent = why;
      el.append(p);
    }
  };

  const send = (m: unknown) => {
    if (!closed) port?.postMessage(m);
  };

  const answer = async (r: Request): Promise<unknown> => {
    const p = r.params ?? {};
    const { connectionId, workspace } = here();
    const conn = k.pluginConnection(connectionId);
    switch (r.method) {
      case 'call': {
        const method = String(p.method);
        const why = panelRefusal(method, p.params, scope);
        if (why) throw { code: 'permission_denied', message: why };
        const params = method.includes('/') ? panelMethodParams(p.params, workspace) : p.params;
        const result = (await conn.call(method as never, params as never)) as unknown;
        if (method === 'workspace.list') {
          // Panels see only the tab's workspace.
          const list = (result as { workspaces: { id: string }[] }).workspaces;
          return { workspaces: list.filter((w) => w.id === workspace) };
        }
        return result;
      }
      case 'subscribe': {
        const topic = String(p.topic);
        const why = topicRefusal(topic, scope);
        if (why) throw { code: 'permission_denied', message: why };
        const sub = ++nextSub;
        subs.set(sub, conn.subscribe(topic, Number(p.after ?? 0), (event) => send({ type: 'event', sub, event })));
        return { sub };
      }
      case 'watch': {
        const a = p.address as { connection?: string; workspace?: string; thread?: string } | undefined;
        if (workspace === undefined || !a || a.connection !== connectionId || a.workspace !== workspace || typeof a.thread !== 'string' || scope.threadWorkspace(a.thread) !== workspace) {
          throw { code: 'permission_denied', message: 'only threads of the tab’s workspace' };
        }
        const sub = ++nextSub;
        subs.set(sub, k.threads.watch({ connection: connectionId, workspace, thread: a.thread }, (view) => send({ type: 'thread', sub, view })));
        return { sub };
      }
      case 'unsubscribe':
      case 'unwatch': {
        const sub = Number(p.sub);
        subs.get(sub)?.();
        subs.delete(sub);
        return {};
      }
      case 'storage.get':
        return k.settings.value.pluginStorage[plugin.id]?.[String(p.key)];
      case 'storage.set':
      case 'storage.delete': {
        const store = { ...(k.settings.value.pluginStorage[plugin.id] ?? {}) };
        if (r.method === 'storage.set') store[String(p.key)] = String(p.value);
        else delete store[String(p.key)];
        k.settings.update({ pluginStorage: { ...k.settings.value.pluginStorage, [plugin.id]: store } });
        return {};
      }
      case 'notify': {
        const tone = p.tone === 'success' || p.tone === 'warning' || p.tone === 'danger' ? p.tone : 'info';
        k.notify(`${plugin.id}: ${String(p.message).slice(0, 300)}`, tone);
        return {};
      }
      case 'commands.run': {
        const id = String(p.id);
        if (!id.startsWith(`${plugin.id}.`)) throw { code: 'permission_denied', message: 'panels run only their own commands' };
        return (await k.commands.run(id, p.args)) ?? null;
      }
      default:
        throw { code: 'method_not_found', message: `no bridge method ${r.method}` };
    }
  };

  const onPortMessage = (e: MessageEvent) => {
    let size = 0;
    try {
      size = JSON.stringify(e.data).length;
    } catch {
      return;
    }
    const r = e.data as Request;
    if (typeof r !== 'object' || r === null || typeof r.id !== 'number' || typeof r.method !== 'string') return;
    if (size > MAX_MESSAGE) {
      send({ id: r.id, error: { code: 'invalid_request', message: 'the message is over 1 MiB' } });
      return;
    }
    if (inFlight >= MAX_IN_FLIGHT) {
      send({ id: r.id, error: { code: 'invalid_request', message: 'too many calls in flight' } });
      return;
    }
    inFlight++;
    answer(r).then(
      (result) => send({ id: r.id, result: result ?? null }),
      (err: unknown) => {
        const e2 = err as { code?: unknown; message?: unknown; data?: { code?: unknown } };
        const code = typeof e2?.code === 'string' ? e2.code : typeof e2?.data?.code === 'string' ? e2.data.code : 'internal';
        send({ id: r.id, error: { code, message: String(e2?.message ?? err) } });
      },
    ).finally(() => inFlight--);
  };

  frame.addEventListener('load', () => {
    loads++;
    if (loads > 1) {
      // It loaded another document: it must not keep the bridge.
      close('This panel navigated away and was closed.');
      return;
    }
    const channel = new MessageChannel();
    port = channel.port1;
    port.onmessage = onPortMessage;
    // The frame's origin is opaque, so the target is '*'; it goes to this window, once.
    frame.contentWindow?.postMessage({ type: 'kumikit.panel', version: 1 }, '*', [channel.port2]);
    send({
      type: 'hello',
      hello: {
        slot,
        contribution: contribution.id,
        address: tab ? { connection: tab.connection, workspace: tab.workspace, thread: tab.thread } : { connection: here().connectionId },
        theme: { tokens: { ...k.theme.tokens }, appearance: k.theme.appearance },
      },
    });
    cleanups.push(k.theme.onChange(() => send({ type: 'theme', tokens: { ...k.theme.tokens }, appearance: k.theme.appearance })));
  });
  frame.src = k.platform.pluginUrl(plugin.id, plugin.entry);
  el.append(frame);
  return () => close();
}
