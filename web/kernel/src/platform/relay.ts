/**
 * Test-only adapter: speaks the shell bridge's line protocol over a WebSocket to
 * `test-plugins/web-relay`, which sits in front of a real core. Only the browser-test
 * bundle contains it.
 */
import type { ClientPluginEntry, Incoming, Platform } from './index';
import { ConnectError, safeModeFromUrl } from './index';

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(path, init);
  if (!r.ok) throw new Error(await r.text());
  return (await r.json()) as T;
}

export function relayPlatform(): Platform {
  return {
    kind: 'relay',
    connect(profile, on) {
      return new Promise((resolve, reject) => {
        const url = new URL(`/bridge?profile=${encodeURIComponent(profile)}`, location.href);
        url.protocol = url.protocol === 'https:' ? 'wss:' : 'ws:';
        const ws = new WebSocket(url);
        // The relay's first message says whether connecting worked, as bridge_connect would.
        let state: 'opening' | 'open' | 'failed' = 'opening';
        ws.onmessage = (e) => {
          const m = JSON.parse(String(e.data)) as Incoming | { type: 'open' } | { type: 'error'; code: string; message: string };
          if (state === 'opening') {
            if (m.type === 'open') {
              state = 'open';
              resolve({ send: async (line) => ws.send(line), close: () => ws.close() });
            } else {
              state = 'failed';
              reject(m.type === 'error' ? new ConnectError(m.code, m.message) : new ConnectError('unreachable', 'the relay did not connect'));
            }
            return;
          }
          if (state === 'open') on(m as Incoming);
        };
        ws.onclose = () => {
          if (state === 'open') on({ type: 'closed', reason: 'the relay closed the connection' });
          else if (state === 'opening') reject(new ConnectError('unreachable', 'cannot reach the relay'));
        };
      });
    },
    pickFile: async () => (await json<{ path: string | null }>('/test/pick')).path,
    pickFolder: async () => (await json<{ path: string | null }>('/test/pick')).path,
    pluginUrl: (id, path) => `/plugins/${id}/${path.replace(/^\/+/, '')}`,
    clientPlugins: {
      list: () => json<ClientPluginEntry[]>('/client-plugins'),
      install: (path) =>
        json<ClientPluginEntry>('/client-plugins', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ path }),
        }),
      remove: async (id) => {
        await fetch(`/client-plugins/${encodeURIComponent(id)}`, { method: 'DELETE' });
      },
    },
    safeMode: safeModeFromUrl(location.href),
  };
}
