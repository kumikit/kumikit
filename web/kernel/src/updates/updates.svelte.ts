/**
 * Application update manager (ADR 0031): displays the current version, checks the signed
 * release manifest on demand or (when opted in) periodically, downloads & verifies updates,
 * and applies automatic updates only after `core.activity` reports every agent is idle.
 */
import type { CoreConnection, Timers } from '../connections/connection.svelte';
import {
  asConnectError,
  type AppUpdateCheckResult,
  type AppUpdateDownloadResult,
  type AppUpdateInfo,
  type Platform,
} from '../platform/index';
import type { SettingsStore } from '../settings/settings.svelte';

export type UpdatePhase =
  | 'idle'
  | 'checking'
  | 'up_to_date'
  | 'available'
  | 'downloading'
  | 'ready'
  | 'waiting_idle'
  | 'applying'
  | 'applied'
  | 'error';

/** Default interval between opt-in background checks: 6 hours. */
export const AUTO_CHECK_INTERVAL_MS = 6 * 60 * 60 * 1000;
/** Default interval between idle polls while an automatic update is staged: 5 seconds. */
export const IDLE_POLL_INTERVAL_MS = 5_000;

const realTimers: Timers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>),
};

export interface UpdateManagerOptions {
  timers?: Timers;
  checkIntervalMs?: number;
  idlePollMs?: number;
  notify?: (message: string, tone: 'info' | 'success' | 'warning' | 'danger') => void;
}

export class UpdateManager {
  info = $state.raw<AppUpdateInfo | undefined>(undefined);
  phase = $state<UpdatePhase>('idle');
  checkResult = $state.raw<AppUpdateCheckResult | undefined>(undefined);
  downloaded = $state.raw<AppUpdateDownloadResult | undefined>(undefined);
  appliedMessage = $state<string | undefined>(undefined);
  packagePath = $state<string | undefined>(undefined);
  error = $state<string | undefined>(undefined);
  lastCheckedAt = $state<string | undefined>(undefined);
  activeTurns = $state(0);
  readyQueued = $state(0);

  readonly #platform: Platform;
  readonly #settings: SettingsStore;
  readonly #homeConnection: () => CoreConnection | undefined;
  readonly #timers: Timers;
  readonly #checkIntervalMs: number;
  readonly #idlePollMs: number;
  readonly #notify: (message: string, tone: 'info' | 'success' | 'warning' | 'danger') => void;

  #checkTimer: unknown;
  #idleTimer: unknown;
  #started = false;
  #stopped = false;
  #busy = false;

  constructor(
    platform: Platform,
    settings: SettingsStore,
    homeConnection: () => CoreConnection | undefined,
    options: UpdateManagerOptions = {},
  ) {
    this.#platform = platform;
    this.#settings = settings;
    this.#homeConnection = homeConnection;
    this.#timers = options.timers ?? realTimers;
    this.#checkIntervalMs = options.checkIntervalMs ?? AUTO_CHECK_INTERVAL_MS;
    this.#idlePollMs = options.idlePollMs ?? IDLE_POLL_INTERVAL_MS;
    this.#notify = options.notify ?? (() => {});
  }

  get supported(): boolean {
    return this.#platform.updates !== undefined;
  }

  get autoUpdate(): boolean {
    return this.#settings.value.autoUpdate;
  }

  get hasAttention(): boolean {
    return (
      this.phase === 'available' ||
      this.phase === 'ready' ||
      this.phase === 'waiting_idle' ||
      this.phase === 'downloading'
    );
  }

  get activeWorkCount(): number {
    return this.activeTurns + this.readyQueued;
  }

  /** Loads local version info (no network). Starts opt-in auto-checks only if enabled. */
  async start(): Promise<void> {
    if (this.#started) return;
    this.#started = true;
    this.#stopped = false;
    if (!this.#platform.updates) return;
    try {
      this.info = await this.#platform.updates.info();
      if (this.info.stagedVersion && this.info.stagedVersion !== this.info.currentVersion) {
        this.downloaded = {
          version: this.info.stagedVersion,
          path: '',
          canAutoInstall: this.info.canAutoInstall,
          readyToInstall: true,
        };
        this.phase = this.autoUpdate && this.info.canAutoInstall ? 'waiting_idle' : 'ready';
      }
    } catch {
      // Info read is best-effort; manual check can still run.
    }
    if (this.autoUpdate) {
      this.#scheduleNextCheck(0);
      if (this.downloaded && this.info?.canAutoInstall) {
        this.#scheduleIdlePoll(0);
      }
    }
  }

  stop(): void {
    this.#stopped = true;
    this.#clearTimers();
  }

  /** Turns opt-in automatic updates on or off. */
  setAutoUpdate(enabled: boolean): void {
    this.#settings.update({ autoUpdate: enabled });
    this.#clearTimers();
    if (!enabled || !this.#platform.updates) {
      if (this.phase === 'waiting_idle' && this.downloaded) {
        this.phase = 'ready';
      }
      return;
    }
    if (this.downloaded && this.info?.canAutoInstall) {
      this.phase = 'waiting_idle';
      this.#scheduleIdlePoll(0);
    } else {
      this.#scheduleNextCheck(0);
    }
  }

  /** Checks the signed release manifest for updates. */
  async checkNow(options: { background?: boolean } = {}): Promise<AppUpdateCheckResult | undefined> {
    if (!this.#platform.updates || this.#busy) return undefined;
    this.#busy = true;
    this.error = undefined;
    if (!options.background || this.phase === 'idle' || this.phase === 'error' || this.phase === 'up_to_date') {
      this.phase = 'checking';
    }
    try {
      const res = await this.#platform.updates.check();
      this.checkResult = res;
      this.lastCheckedAt = new Date().toISOString();
      if (!res.updateAvailable) {
        this.phase = 'up_to_date';
        return res;
      }
      if (this.downloaded?.version === res.latestVersion) {
        this.phase = this.autoUpdate && res.canAutoInstall ? 'waiting_idle' : 'ready';
      } else {
        this.phase = 'available';
      }
      if (this.autoUpdate && res.canAutoInstall && res.assetAvailable) {
        this.#busy = false;
        await this.download({ autoApplyWhenIdle: true });
        return res;
      }
      return res;
    } catch (e) {
      const err = asConnectError(e);
      this.error = err.message;
      this.phase = 'error';
      if (!options.background) {
        this.#notify(`Update check failed: ${err.message}`, 'danger');
      }
      return undefined;
    } finally {
      this.#busy = false;
      if (this.autoUpdate && !this.#stopped) {
        this.#scheduleNextCheck(this.#checkIntervalMs);
      }
    }
  }

  /** Downloads and SHA-256-verifies the latest platform installer. */
  async download(options: { autoApplyWhenIdle?: boolean } = {}): Promise<AppUpdateDownloadResult | undefined> {
    if (!this.#platform.updates || this.#busy) return undefined;
    this.#busy = true;
    this.error = undefined;
    this.phase = 'downloading';
    try {
      const res = await this.#platform.updates.download();
      this.downloaded = res;
      this.info = await this.#platform.updates.info().catch(() => this.info);
      if (options.autoApplyWhenIdle && res.canAutoInstall) {
        this.phase = 'waiting_idle';
        this.#busy = false;
        await this.tryApplyWhenIdle();
        return res;
      }
      this.phase = 'ready';
      return res;
    } catch (e) {
      const err = asConnectError(e);
      this.error = err.message;
      this.phase = 'error';
      this.#notify(`Update download failed: ${err.message}`, 'danger');
      return undefined;
    } finally {
      this.#busy = false;
    }
  }

  /**
   * Manual install action: downloads if needed, then applies (`ifIdle` defaults to false for
   * explicit user clicks, or true if waiting for agents).
   */
  async installNow(options: { ifIdle?: boolean } = {}): Promise<boolean> {
    if (!this.#platform.updates) return false;
    if (!this.downloaded || (this.checkResult && this.downloaded.version !== this.checkResult.latestVersion)) {
      const dl = await this.download({ autoApplyWhenIdle: false });
      if (!dl) return false;
    }
    const version = this.downloaded?.version;
    if (!version) return false;
    return this.#apply(version, options.ifIdle ?? false);
  }

  /**
   * Checks whether all agents on the home core are idle; if so, applies the staged update
   * with `ifIdle: true`. Otherwise stays in `waiting_idle` and schedules another check.
   */
  async tryApplyWhenIdle(): Promise<boolean> {
    if (this.#stopped || !this.#platform.updates || !this.autoUpdate) return false;
    const version = this.downloaded?.version;
    if (!version || !this.downloaded?.canAutoInstall) return false;

    const conn = this.#homeConnection();
    if (!conn || !conn.ready) {
      this.phase = 'waiting_idle';
      this.#scheduleIdlePoll(this.#idlePollMs);
      return false;
    }

    try {
      const activity = await conn.call('core.activity', {});
      this.activeTurns = activity.activeTurns;
      this.readyQueued = activity.readyQueued;
      if (!activity.idle) {
        this.phase = 'waiting_idle';
        this.#scheduleIdlePoll(this.#idlePollMs);
        return false;
      }
    } catch {
      this.phase = 'waiting_idle';
      this.#scheduleIdlePoll(this.#idlePollMs);
      return false;
    }

    const ok = await this.#apply(version, true);
    if (!ok && this.phase === 'waiting_idle' && this.autoUpdate && !this.#stopped) {
      this.#scheduleIdlePoll(this.#idlePollMs);
    }
    return ok;
  }

  async #apply(version: string, ifIdle: boolean): Promise<boolean> {
    if (!this.#platform.updates || this.#busy) return false;
    this.#busy = true;
    this.error = undefined;
    this.phase = 'applying';
    try {
      const res = await this.#platform.updates.apply(version, ifIdle);
      this.appliedMessage = res.message;
      this.packagePath = res.packagePath;
      this.phase = 'applied';
      this.#clearTimers();
      this.#notify(res.message, 'success');
      return true;
    } catch (e) {
      const err = asConnectError(e);
      if (err.code === 'busy') {
        this.phase = 'waiting_idle';
        return false;
      }
      this.error = err.message;
      this.phase = 'error';
      this.#notify(`Update install failed: ${err.message}`, 'danger');
      return false;
    } finally {
      this.#busy = false;
    }
  }

  #scheduleNextCheck(delayMs: number): void {
    this.#timers.clear(this.#checkTimer);
    if (this.#stopped || !this.autoUpdate) return;
    this.#checkTimer = this.#timers.set(() => {
      void this.checkNow({ background: true });
    }, delayMs);
  }

  #scheduleIdlePoll(delayMs: number): void {
    this.#timers.clear(this.#idleTimer);
    if (this.#stopped || !this.autoUpdate) return;
    this.#idleTimer = this.#timers.set(() => {
      void this.tryApplyWhenIdle();
    }, delayMs);
  }

  #clearTimers(): void {
    this.#timers.clear(this.#checkTimer);
    this.#timers.clear(this.#idleTimer);
    this.#checkTimer = undefined;
    this.#idleTimer = undefined;
  }
}
