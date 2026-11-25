/** The desktop shell's bridge (ADR 0013). */
import { Channel, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';

import type {
  AppUpdateApplyResult,
  AppUpdateCheckResult,
  AppUpdateDownloadResult,
  AppUpdateInfo,
  ClientPluginEntry,
  ClientRegistryResult,
  ClientUpdate,
  Incoming,
  PickedAttachment,
  Platform,
  ProfileInfo,
  Removed,
  SshProbeResult,
  SshSyncResult,
} from './index';
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
      updateSsh: (request) => call<ProfileInfo>('profile_update_ssh', { request }),
      probeSsh: (request) => call<SshProbeResult>('ssh_probe', { request }),
      syncSsh: (request) => call<SshSyncResult>('ssh_sync', { request }),
      remove: (name) => call<Removed>('profile_remove', { name }),
    },
    pickBytes: (accept) => pickWithInput(accept),
    pickAttachments: (title, defaultPath) =>
      call<PickedAttachment[]>('pick_attachments', defaultPath ? { title, defaultPath } : { title }),
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
    registry: {
      search: (query, refresh) => call<ClientRegistryResult[]>('client_registry_search', { query, refresh }),
      install: (plugin) => call<ClientPluginEntry>('client_plugins_install_registry', { plugin }),
      updates: (refresh) => call<ClientUpdate[]>('client_plugins_updates', { refresh }),
      update: (id, version) => call<ClientPluginEntry>('client_plugins_update', { id, version }),
      registries: () => call<string[]>('client_registries_get'),
      setRegistries: (urls) => call<void>('client_registries_set', { urls }),
      lock: () => call<string>('client_lock'),
    },
    updates: {
      info: () => call<AppUpdateInfo>('app_update_info'),
      check: () => call<AppUpdateCheckResult>('app_update_check'),
      download: () => call<AppUpdateDownloadResult>('app_update_download'),
      apply: (version, ifIdle) => call<AppUpdateApplyResult>('app_update_apply', { request: { version, ifIdle } }),
    },
    safeMode: safeModeFromUrl(location.href),
  };
}
