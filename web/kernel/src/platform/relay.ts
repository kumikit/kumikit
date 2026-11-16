/**
 * Test-only adapter: speaks the shell bridge's line protocol over a WebSocket to
 * `test-plugins/web-relay`, which sits in front of a real core. Only the browser-test
 * bundle contains it.
 */
import type { ClientPluginEntry, Platform } from './index';
import { safeModeFromUrl } from './index';

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
        let open = false;
        ws.onopen = () => {
          open = true;
          resolve({
            send: async (line) => ws.send(line),
            close: () => ws.close(),
          });
        };
        ws.onmessage = (e) => on(JSON.parse(String(e.data)));
        ws.onclose = () => {
          if (open) on({ type: 'closed', reason: 'the relay closed the connection' });
          else reject(new Error('cannot reach the relay'));
        };
      });
    },
    pickFile: async () => (await json<{ path: string | null }>('/test/pick')).path,
    pickFolder: async () => (await json<{ path: string | null }>('/test/pick')).path,
    pluginUrl: (id, path) => `/plugins/${id}/${path.replace(/^\/+/, '')}`,
    clientPlugins: {
      list: () => json<ClientPluginEntry[]>('/client-plugins'),
      install: (path) =>
        json<ClientPluginEntry>('/client-plugins', { method: 'POST', body: JSON.stringify({ path }) }),
      remove: async (id) => {
        await fetch(`/client-plugins/${encodeURIComponent(id)}`, { method: 'DELETE' });
      },
    },
    safeMode: safeModeFromUrl(location.href),
  };
}
