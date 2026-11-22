/**
 * What the UI kernel needs from where it runs (06-client): the desktop shell, a browser
 * served by a core, or the test relay in front of real cores.
 */

/** What arrives on a connection, in order (the shell bridge's `Incoming`). */
export type Incoming = { type: 'line'; line: string } | { type: 'closed'; reason: string };

/**
 * Why connecting failed, as the shell's `bridge_connect` reports it: `unauthorized` (the
 * device was revoked), `certificate_changed`, `unreachable`, `unknown_profile`, … The UI
 * stops retrying on the first two.
 */
export class ConnectError extends Error {
  constructor(
    readonly code: string,
    message: string,
  ) {
    super(message);
  }
}

/** An open line connection to a core. */
export interface LineConnection {
  send(line: string): Promise<void>;
  close(): void;
}

/** A UI plugin or theme as the client plugin store lists it. */
export interface ClientPluginEntry {
  id: string;
  version: string;
  /** Bundled with the app (read-only) or installed by the user. */
  source: 'bundled' | 'installed';
  /** The manifest, as JSON. The UI kernel checks its contents when it loads it. */
  manifest: unknown;
  /** Who signed it, and the registry it came from (10-distribution). */
  publisher?: { key: string; registry?: string };
}

/** What a UI plugin needs: how its code runs, the hosts it may contact, its core plugins. */
export interface UiNeeds {
  isolation?: string;
  network?: string[];
  requiresCore?: string[];
}

/** A UI plugin or theme a registry offers (the shell's `client_registry_search`). */
export interface ClientRegistryResult {
  id: string;
  name: string;
  description?: string;
  kind: string[];
  publisher: string;
  key: string;
  registry: string;
  registryName: string;
  repository?: string;
  version: string;
  ui: UiNeeds;
  starter: boolean;
}

/** An update a registry offers for an installed UI plugin or theme. */
export interface ClientUpdate {
  plugin: string;
  from: string;
  to: string;
  registry: string;
  ui: UiNeeds;
  /** It needs more than the installed version: new hosts, or inline code. */
  widens: boolean;
}

/** A plugin to install from the client's registries. */
export interface ClientRef {
  id: string;
  version?: string;
  sha256?: string;
  registry?: string;
}

/** A connection profile as the shell shows it: no address, no token. */
export interface ProfileInfo {
  /** The connection id in addresses. */
  name: string;
  label: string;
  /** `local` (this computer), `tls` (paired), `ssh`, or `core` (the core serving a browser). */
  kind: 'local' | 'tls' | 'ssh' | 'core';
}

export interface PairRequest {
  name: string;
  label: string;
  /** `host[:port]`, or the URL `kumikit pair` printed. */
  address: string;
  /** The fingerprint the user confirmed. */
  fingerprint: string;
  code: string;
  deviceName?: string;
}

export interface SshRequest {
  name: string;
  label: string;
  destination: string;
  command?: string;
}

export interface Removed {
  /** Whether this device was revoked on the core. */
  revoked: boolean;
  /** The device id to revoke there by hand, if not. */
  deviceId?: string;
}

/** A file the user picked, read in the page. */
export interface PickedFile {
  name: string;
  bytes: Uint8Array;
}

/** A file picked for attachment via a native OS file dialog rooted at a project directory. */
export interface PickedAttachment {
  name: string;
  mimeType: string;
  data: string;
  size: number;
}

export interface Platform {
  readonly kind: 'tauri' | 'relay' | 'browser';
  /** The profile that always exists: `local` on the desktop, `core` in a browser. */
  readonly home: string;
  /** Opens a connection to a profile's core. Rejects with a {@link ConnectError}. */
  connect(profile: string, on: (m: Incoming) => void): Promise<LineConnection>;
  /** Connection profiles. Rejects with a {@link ConnectError}. */
  profiles: {
    /** Whether cores can be added and removed here (not in a browser). */
    readonly editable: boolean;
    list(): Promise<ProfileInfo[]>;
    /** The certificate fingerprint of the core at an address. */
    probe(address: string): Promise<string>;
    pair(request: PairRequest): Promise<ProfileInfo>;
    addSsh(request: SshRequest): Promise<ProfileInfo>;
    remove(name: string): Promise<Removed>;
  };
  /** A file's contents, picked with a file input (for uploads to a core). */
  pickBytes(accept: string): Promise<PickedFile | null>;
  /** Multi-file native picker rooted at `defaultPath` when supported (desktop shell). */
  pickAttachments?(title: string, defaultPath?: string): Promise<PickedAttachment[]>;
  /** A path on the core's machine, or null if the user cancelled. */
  pickFile(title: string, extensions: string[]): Promise<string | null>;
  pickFolder(title: string): Promise<string | null>;
  /** The URL of a file inside a UI plugin. */
  pluginUrl(id: string, path: string): string;
  clientPlugins: {
    list(): Promise<ClientPluginEntry[]>;
    install(path: string): Promise<ClientPluginEntry>;
    remove(id: string): Promise<void>;
  };
  /** UI plugins and themes from registries, through the shell (absent in a browser).
   * Failures are {@link ConnectError}s with the shell's codes (`unavailable`,
   * `untrusted`, `invalid`, `not_found`, `conflict`). Nothing is fetched until called. */
  readonly registry?: {
    search(query: string, refresh: boolean): Promise<ClientRegistryResult[]>;
    install(plugin: ClientRef): Promise<ClientPluginEntry>;
    updates(refresh: boolean): Promise<ClientUpdate[]>;
    update(id: string, version: string): Promise<ClientPluginEntry>;
    registries(): Promise<string[]>;
    setRegistries(urls: string[]): Promise<void>;
    /** The client lockfile's text. */
    lock(): Promise<string>;
  };
  /** Whether to start without UI plugins (06-client, safe mode). */
  readonly safeMode: boolean;
  /** Only in a browser served by a core: pairing this browser, and signing out. */
  readonly browser?: {
    /** Pairs with a code from `kumikit pair`; the core keeps the token in an HttpOnly cookie. */
    pair(code: string, deviceName: string): Promise<void>;
    /** Revokes this browser's device on the core. */
    signOut(): Promise<void>;
  };
}

/** Reads a file the user picks with a standard file input: the page gets only the file
 * chosen, through the browser's own picker, and no native command is involved. */
export function pickWithInput(accept: string, doc: Document = document): Promise<PickedFile | null> {
  return new Promise((resolve) => {
    const input = doc.createElement('input');
    input.type = 'file';
    input.accept = accept;
    input.hidden = true;
    input.setAttribute('data-kk-picker', '');
    const done = (file: PickedFile | null) => {
      input.remove();
      resolve(file);
    };
    input.addEventListener('change', () => {
      const f = input.files?.[0];
      if (!f) return done(null);
      void f.arrayBuffer().then(
        (b) => done({ name: f.name, bytes: new Uint8Array(b) }),
        () => done(null),
      );
    });
    input.addEventListener('cancel', () => done(null));
    doc.body.append(input);
    input.click();
  });
}

/** Base64 of bytes, in chunks so large packages do not overflow the call stack. */
export function toBase64(bytes: Uint8Array): string {
  let s = '';
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s);
}

/** Turns what a shell command or the relay rejected with into a {@link ConnectError}. */
export function asConnectError(e: unknown): ConnectError {
  if (e instanceof ConnectError) return e;
  if (typeof e === 'object' && e !== null && 'code' in e && 'message' in e) {
    return new ConnectError(String((e as { code: unknown }).code), String((e as { message: unknown }).message));
  }
  return new ConnectError('internal', e instanceof Error ? e.message : String(e));
}

declare const __KK_PLATFORM__: 'tauri' | 'relay' | 'browser';

/** The adapter this bundle was built for. */
export async function loadPlatform(): Promise<Platform> {
  if (__KK_PLATFORM__ === 'relay') {
    const m = await import('./relay');
    return m.relayPlatform();
  }
  if (__KK_PLATFORM__ === 'browser') {
    const m = await import('./browser');
    return m.browserPlatform();
  }
  const m = await import('./tauri');
  return m.tauriPlatform();
}

/** `?safe` in the start URL. */
export function safeModeFromUrl(url: string): boolean {
  return new URL(url).searchParams.has('safe');
}
