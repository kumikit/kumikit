/**
 * Client settings (06-client, Client settings): kept in the webview's storage, inside
 * the client data directory. Nothing secret is stored here.
 */

export interface TabSpec {
  connection: string;
  workspace: string;
  /** The thread open in the tab, if any. */
  thread?: string;
}

export interface Settings {
  /** `<theme>:<variant>`, or absent to follow the appearance. */
  themeVariant?: string;
  appearance: 'system' | 'light' | 'dark';
  reduceMotion: boolean;
  tabs: TabSpec[];
  activeTab: number;
  /** UI plugins the user disabled (bundled ones are enabled unless listed). */
  disabledPlugins: string[];
  /** Installed inline UI plugins the user trusts. */
  trustedPlugins: string[];
  /** Per-plugin storage (`ctx.storage`). */
  pluginStorage: Record<string, Record<string, string>>;
  /** The first-run welcome was answered (10-distribution, Starter pack). */
  welcomeDone: boolean;
}

export const DEFAULTS: Settings = {
  appearance: 'system',
  reduceMotion: false,
  tabs: [],
  activeTab: 0,
  disabledPlugins: [],
  trustedPlugins: [],
  pluginStorage: {},
  welcomeDone: false,
};

const KEY = 'kumikit.settings';

/** A small key-value store; `localStorage` in the page. */
export interface Storage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

/** Reads settings, keeping only known fields of the right type. */
export function parseSettings(text: string | null): Settings {
  let raw: unknown;
  try {
    raw = text ? JSON.parse(text) : {};
  } catch {
    raw = {};
  }
  const r = (typeof raw === 'object' && raw !== null ? raw : {}) as Partial<Record<keyof Settings, unknown>>;
  const strings = (v: unknown) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : []);
  const s: Settings = {
    ...DEFAULTS,
    appearance: r.appearance === 'light' || r.appearance === 'dark' ? r.appearance : 'system',
    reduceMotion: r.reduceMotion === true,
    tabs: Array.isArray(r.tabs)
      ? (r.tabs as unknown[]).filter(
          (t): t is TabSpec =>
            typeof t === 'object' && t !== null && typeof (t as TabSpec).connection === 'string' && typeof (t as TabSpec).workspace === 'string',
        )
      : [],
    activeTab: typeof r.activeTab === 'number' ? r.activeTab : 0,
    disabledPlugins: strings(r.disabledPlugins),
    trustedPlugins: strings(r.trustedPlugins),
    pluginStorage:
      typeof r.pluginStorage === 'object' && r.pluginStorage !== null ? (r.pluginStorage as Settings['pluginStorage']) : {},
    welcomeDone: r.welcomeDone === true,
  };
  if (typeof r.themeVariant === 'string') s.themeVariant = r.themeVariant;
  return s;
}

export class SettingsStore {
  value = $state.raw<Settings>(DEFAULTS);

  constructor(private readonly storage: Storage | undefined) {
    this.value = parseSettings(storage?.getItem(KEY) ?? null);
  }

  update(change: Partial<Settings>): void {
    this.value = { ...this.value, ...change };
    try {
      this.storage?.setItem(KEY, JSON.stringify(this.value));
    } catch {
      // Storage full or unavailable: the setting holds for this session.
    }
  }
}
