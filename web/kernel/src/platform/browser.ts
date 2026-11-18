/**
 * A plain browser served by a core (06-client, Browser client): one connection, to the
 * core that served the page, over `wss://<this host>/rpc`. The core authenticates the
 * socket by the HttpOnly cookie it set at pairing; this code never sees the token.
 */
import type { ClientPluginEntry, Platform } from './index';
import { ConnectError, pickWithInput, safeModeFromUrl } from './index';

/** The connection id of the core serving the page. */
export const CORE = 'core';

async function status(): Promise<{ paired: boolean; core: { name: string } }> {
  const r = await fetch('/auth/status', { cache: 'no-store' });
  if (!r.ok) throw new ConnectError('unreachable', `the core answered ${r.status}`);
  return (await r.json()) as { paired: boolean; core: { name: string } };
}

async function post(path: string, body: unknown): Promise<Response> {
  return fetch(path, { method: 'POST', headers: { 'content-type': 'application/json' }, body: JSON.stringify(body) });
}

const unavailable = () => Promise.reject(new ConnectError('invalid', 'cores are added in the desktop app'));

export function browserPlatform(): Platform {
  return {
    kind: 'browser',
    home: CORE,
    connect(_profile, on) {
      return new Promise((resolve, reject) => {
        // `ws:` only from the dev server on localhost (scripts/dev-browser.mjs).
        const ws = new WebSocket(`${location.protocol === 'https:' ? 'wss' : 'ws'}://${location.host}/rpc`);
        let open = false;
        // A socket can stay "connecting" without ever failing (a core that stopped
        // answering); give up after 10 s so the reconnect backoff keeps going.
        const timer = setTimeout(() => {
          if (!open) ws.close();
        }, 10_000);
        ws.onopen = () => {
          clearTimeout(timer);
          open = true;
          resolve({ send: async (line) => ws.send(line), close: () => ws.close() });
        };
        ws.onmessage = (e) => on({ type: 'line', line: String(e.data) });
        ws.onclose = (e) => {
          clearTimeout(timer);
          if (open) {
            on({ type: 'closed', reason: e.code === 4001 ? 'this device was revoked' : 'the core closed the connection' });
            return;
          }
          // A browser does not say why an upgrade failed; ask whether this browser is paired.
          status().then(
            (s) => reject(s.paired ? new ConnectError('unreachable', 'the core refused the connection') : new ConnectError('unauthorized', 'this browser is not paired with the core')),
            (err: unknown) => reject(err instanceof ConnectError ? err : new ConnectError('unreachable', 'cannot reach the core')),
          );
        };
      });
    },
    profiles: {
      editable: false,
      list: async () => [{ name: CORE, label: (await status().catch(() => undefined))?.core.name ?? 'This core', kind: 'core' }],
      probe: unavailable,
      pair: unavailable,
      addSsh: unavailable,
      remove: unavailable,
    },
    pickBytes: (accept) => pickWithInput(accept),
    // Neither picker can show the core's files; the UI asks for typed paths instead.
    pickFile: async () => null,
    pickFolder: async () => null,
    pluginUrl: (id, path) => `/ui-plugins/${encodeURIComponent(id)}/${path.replace(/^\/+/, '')}`,
    clientPlugins: {
      async list() {
        const r = await fetch('/ui-plugins/index.json', { cache: 'no-store' });
        return r.ok ? ((await r.json()) as ClientPluginEntry[]) : [];
      },
      install: () => Promise.reject(new Error('UI plugins are installed in the desktop app')),
      remove: () => Promise.reject(new Error('UI plugins are removed in the desktop app')),
    },
    safeMode: safeModeFromUrl(location.href),
    browser: {
      async pair(code, deviceName) {
        const r = await post('/auth/pair', { code, deviceName });
        if (!r.ok) {
          const body = (await r.json().catch(() => ({}))) as { message?: string };
          throw new ConnectError(r.status === 401 ? 'unauthorized' : 'invalid', body.message ?? `the core answered ${r.status}`);
        }
      },
      async signOut() {
        await post('/auth/logout', {});
      },
    },
  };
}
