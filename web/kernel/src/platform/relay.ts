/**
 * Test-only adapter: speaks the shell bridge's line protocol over a WebSocket to
 * `test-plugins/web-relay`, which sits in front of a real core. Only the browser-test
 * bundle contains it.
 */
import type { ClientPluginEntry, Incoming, Platform, ProfileInfo, Removed } from './index';
import { ConnectError, asConnectError, pickWithInput, safeModeFromUrl } from './index';

/** A profile endpoint: failures carry the shell's `{ code, message }`. */
async function profileCall<T>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(path, init);
  const body: unknown = await r.json().catch(() => ({ code: 'internal', message: r.statusText }));
  if (!r.ok) throw asConnectError(body);
  return body as T;
}

const post = (body: unknown): RequestInit => ({
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify(body),
});

async function json<T>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(path, init);
  if (!r.ok) throw new Error(await r.text());
  return (await r.json()) as T;
}

export function relayPlatform(): Platform {
  return {
    kind: 'relay',
    home: 'local',
    profiles: {
      editable: true,
      list: () => profileCall<ProfileInfo[]>('/profiles'),
      probe: (address) => profileCall<string>('/profiles/probe', post({ address })),
      pair: (request) => profileCall<ProfileInfo>('/profiles/pair', post(request)),
      addSsh: (request) => profileCall<ProfileInfo>('/profiles/ssh', post(request)),
      remove: (name) => profileCall<Removed>(`/profiles/${encodeURIComponent(name)}`, { method: 'DELETE' }),
    },
    pickBytes: (accept) => pickWithInput(accept),
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
