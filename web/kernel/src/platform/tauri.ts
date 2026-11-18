/** The desktop shell's bridge (ADR 0013). */
import { Channel, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

import type { ClientPluginEntry, Incoming, Platform, ProfileInfo, Removed } from './index';
import { asConnectError, pickWithInput, safeModeFromUrl } from './index';

/** Invokes a shell command; its `{ code, message }` failures become ConnectErrors. */
async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    throw asConnectError(e);
  }
}

export function tauriPlatform(): Platform {
  const windows = navigator.userAgent.includes('Windows');
  return {
    kind: 'tauri',
    home: 'local',
    async connect(profile, on) {
      const channel = new Channel<Incoming>();
      channel.onmessage = on;
      const connection = await call<string>('bridge_connect', { profile, channel });
      return {
        send: (line) => invoke<void>('bridge_send', { connection, line }),
        close: () => void invoke('bridge_close', { connection }),
      };
    },
    profiles: {
      editable: true,
      list: () => call<ProfileInfo[]>('profiles_list'),
      probe: (address) => call<string>('profile_probe', { address }),
      pair: (request) => call<ProfileInfo>('profile_pair', { request }),
      addSsh: (request) => call<ProfileInfo>('profile_add_ssh', { request }),
      remove: (name) => call<Removed>('profile_remove', { name }),
    },
    pickBytes: (accept) => pickWithInput(accept),
    async pickFile(title, extensions) {
      const path = await open({ title, multiple: false, directory: false, filters: [{ name: title, extensions }] });
      return typeof path === 'string' ? path : null;
    },
    async pickFolder(title) {
      const path = await open({ title, multiple: false, directory: true });
      return typeof path === 'string' ? path : null;
    },
    pluginUrl(id, path) {
      const rel = path.replace(/^\/+/, '');
      return windows ? `http://kumikit-plugin.localhost/${id}/${rel}` : `kumikit-plugin://${id}/${rel}`;
    },
    clientPlugins: {
      list: () => invoke<ClientPluginEntry[]>('client_plugins_list'),
      install: (path) => invoke<ClientPluginEntry>('client_plugins_install', { path }),
      remove: (id) => invoke<void>('client_plugins_remove', { id }),
    },
    safeMode: safeModeFromUrl(location.href),
  };
}
