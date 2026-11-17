/** The desktop shell's bridge (ADR 0013). */
import { Channel, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

import type { ClientPluginEntry, Incoming, Platform } from './index';
import { safeModeFromUrl } from './index';

export function tauriPlatform(): Platform {
  const windows = navigator.userAgent.includes('Windows');
  return {
    kind: 'tauri',
    async connect(profile, on) {
      const channel = new Channel<Incoming>();
      channel.onmessage = on;
      const connection = await invoke<string>('bridge_connect', { profile, channel });
      return {
        send: (line) => invoke<void>('bridge_send', { connection, line }),
        close: () => void invoke('bridge_close', { connection }),
      };
    },
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
