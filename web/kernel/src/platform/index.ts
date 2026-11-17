/**
 * What the UI kernel needs from where it runs (06-client): the desktop shell, or the
 * test relay in front of a real core. M3 adds a browser WebSocket adapter.
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
}

export interface Platform {
  readonly kind: 'tauri' | 'relay';
  /** Opens a connection to a profile's core (`local` in M2). */
  connect(profile: string, on: (m: Incoming) => void): Promise<LineConnection>;
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
  /** Whether to start without UI plugins (06-client, safe mode). */
  readonly safeMode: boolean;
}

declare const __KK_PLATFORM__: 'tauri' | 'relay';

/** The adapter this bundle was built for. */
export async function loadPlatform(): Promise<Platform> {
  if (__KK_PLATFORM__ === 'relay') {
    const m = await import('./relay');
    return m.relayPlatform();
  }
  const m = await import('./tauri');
  return m.tauriPlatform();
}

/** `?safe` in the start URL. */
export function safeModeFromUrl(url: string): boolean {
  return new URL(url).searchParams.has('safe');
}
