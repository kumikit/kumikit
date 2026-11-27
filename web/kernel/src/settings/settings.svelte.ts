/**
 * Client settings (06-client, Client settings): kept in the webview's storage, inside
 * the client data directory. Nothing secret is stored here.
 */
import type { PermissionMode } from '@kumikit/protocol';
import {
  LEFT_DEFAULT,
  LEFT_MAX,
  LEFT_MIN,
  RIGHT_DEFAULT,
  RIGHT_MAX,
  RIGHT_MIN,
  clamp,
  defaultLayout,
  findLeaf,
  firstLeaf,
  parseLayout,
  type LayoutNode,
  type LeftSidebarView,
} from '../layout/layout';

export interface TabSpec {
  connection: string;
  workspace: string;
  /** The thread open in the tab, if any. */
  thread?: string;
  /** Optional linked Git worktree path for a draft thread in this workspace. */
  worktreePath?: string;
}

export type WindowButtonsMode = 'auto' | 'show' | 'hide';

export interface Settings {
  /** `<theme>:<variant>`, or absent to follow the appearance. */
  themeVariant?: string;
  appearance: 'system' | 'light' | 'dark';
  reduceMotion: boolean;
  /** Mirror of the active pane's tabs (kept for compatibility with legacy storage). */
  tabs: TabSpec[];
  activeTab: number;
  /** Split tree of workspace panes. */
  layout: LayoutNode;
  /** Currently focused leaf pane id. */
  activePaneId: string;
  /** Left sidebar state and width in pixels. */
  leftSidebarOpen: boolean;
  leftSidebarWidth: number;
  leftSidebarView: LeftSidebarView;
  /** Right inspector dock state, width in pixels, open tabs, and selected tab key. */
  rightSidebarOpen: boolean;
  rightSidebarWidth: number;
  /** Open inspector panel keys (`<plugin>/<slotId>`). Absent until the user opens or closes tabs. */
  openInspectorTabs?: string[];
  activeInspectorTab?: string;
  /** Last-used agent provider and model for inline new-thread creation. */
  lastProvider?: string;
  lastModel?: string;
  /** Last-used model settings (effort, speed, …) for inline new-thread creation. */
  lastOptions?: Record<string, string>;
  /** Recently picked provider and model pairs, newest first, per connection. */
  recentModels?: Record<string, { provider: string; model: string }[]>;
  /** Default agent permission mode applied when starting a new thread (default `standard`). */
  defaultPermissionMode: PermissionMode;
  /** UI plugins the user disabled (bundled ones are enabled unless listed). */
  disabledPlugins: string[];
  /** Installed inline UI plugins the user trusts. */
  trustedPlugins: string[];
  /** Per-plugin storage (`ctx.storage`). */
  pluginStorage: Record<string, Record<string, string>>;
  /** The first-run welcome was answered (10-distribution, Starter pack). */
  welcomeDone: boolean;
  /** Opt-in automatic desktop updates, applied only when all agents are idle (default false). */
  autoUpdate: boolean;
  /** Opt-in automatic matching `kumikit` sync on SSH hosts (default false). */
  autoSyncSsh: boolean;
  /** Whether to show custom minimize/maximize/close buttons in the top bar (borderless desktop). */
  windowButtons: WindowButtonsMode;
}

export const DEFAULTS: Settings = {
  appearance: 'system',
  reduceMotion: false,
  tabs: [],
  activeTab: 0,
  layout: defaultLayout(),
  activePaneId: 'pane-1',
  leftSidebarOpen: true,
  leftSidebarWidth: LEFT_DEFAULT,
  leftSidebarView: 'workspaces',
  rightSidebarOpen: false,
  rightSidebarWidth: RIGHT_DEFAULT,
  defaultPermissionMode: 'standard',
  disabledPlugins: [],
  trustedPlugins: [],
  pluginStorage: {},
  welcomeDone: false,
  autoUpdate: false,
  autoSyncSsh: false,
  windowButtons: 'auto',
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
  const legacyTabs = Array.isArray(r.tabs)
    ? (r.tabs as unknown[]).filter(
        (t): t is TabSpec =>
          typeof t === 'object' && t !== null && typeof (t as TabSpec).connection === 'string' && typeof (t as TabSpec).workspace === 'string',
      )
    : [];
  const legacyActiveTab = typeof r.activeTab === 'number' ? r.activeTab : 0;
  const layout = parseLayout(r.layout, legacyTabs, legacyActiveTab);
  const candidatePane = typeof r.activePaneId === 'string' ? findLeaf(layout, r.activePaneId) : undefined;
  const activePane = candidatePane ?? firstLeaf(layout);

  const s: Settings = {
    ...DEFAULTS,
    appearance: r.appearance === 'light' || r.appearance === 'dark' ? r.appearance : 'system',
    reduceMotion: r.reduceMotion === true,
    tabs: activePane.tabs,
    activeTab: activePane.activeTab,
    layout,
    activePaneId: activePane.id,
    leftSidebarOpen: r.leftSidebarOpen === undefined ? DEFAULTS.leftSidebarOpen : r.leftSidebarOpen === true,
    leftSidebarWidth: typeof r.leftSidebarWidth === 'number' ? clamp(r.leftSidebarWidth, LEFT_MIN, LEFT_MAX) : DEFAULTS.leftSidebarWidth,
    leftSidebarView: r.leftSidebarView === 'threads' ? 'threads' : 'workspaces',
    rightSidebarOpen: r.rightSidebarOpen === true,
    rightSidebarWidth: typeof r.rightSidebarWidth === 'number' ? clamp(r.rightSidebarWidth, RIGHT_MIN, RIGHT_MAX) : DEFAULTS.rightSidebarWidth,
    defaultPermissionMode:
      r.defaultPermissionMode === 'standard' ||
      r.defaultPermissionMode === 'accept_edits' ||
      r.defaultPermissionMode === 'plan' ||
      r.defaultPermissionMode === 'full_access'
        ? r.defaultPermissionMode
        : DEFAULTS.defaultPermissionMode,
    disabledPlugins: strings(r.disabledPlugins),
    trustedPlugins: strings(r.trustedPlugins),
    pluginStorage:
      typeof r.pluginStorage === 'object' && r.pluginStorage !== null ? (r.pluginStorage as Settings['pluginStorage']) : {},
    welcomeDone: r.welcomeDone === true,
    autoUpdate: r.autoUpdate === true,
    autoSyncSsh: r.autoSyncSsh === true,
    windowButtons: r.windowButtons === 'show' || r.windowButtons === 'hide' ? r.windowButtons : 'auto',
  };
  if (typeof r.themeVariant === 'string') s.themeVariant = r.themeVariant;
  if (Array.isArray(r.openInspectorTabs)) s.openInspectorTabs = strings(r.openInspectorTabs);
  if (typeof r.activeInspectorTab === 'string') s.activeInspectorTab = r.activeInspectorTab;
  if (typeof r.lastProvider === 'string') s.lastProvider = r.lastProvider;
  if (typeof r.lastModel === 'string') s.lastModel = r.lastModel;
  if (typeof r.lastOptions === 'object' && r.lastOptions !== null) {
    const o = Object.entries(r.lastOptions as Record<string, unknown>).filter((e): e is [string, string] => typeof e[1] === 'string');
    if (o.length > 0) s.lastOptions = Object.fromEntries(o);
  }
  if (typeof r.recentModels === 'object' && r.recentModels !== null) {
    const recent: Record<string, { provider: string; model: string }[]> = {};
    for (const [conn, list] of Object.entries(r.recentModels as Record<string, unknown>)) {
      if (!Array.isArray(list)) continue;
      recent[conn] = list.filter(
        (x): x is { provider: string; model: string } =>
          typeof x === 'object' && x !== null && typeof (x as { provider?: unknown }).provider === 'string' && typeof (x as { model?: unknown }).model === 'string',
      ).map((x) => ({ provider: x.provider, model: x.model }));
    }
    s.recentModels = recent;
  }
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
