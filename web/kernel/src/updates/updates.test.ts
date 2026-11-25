import { describe, expect, it, vi } from 'vitest';
import type { CoreConnection, Timers } from '../connections/connection.svelte';
import { ConnectError, type AppUpdateCheckResult, type Platform } from '../platform/index';
import { SettingsStore, parseSettings } from '../settings/settings.svelte';
import { UpdateManager } from './updates.svelte';

class ManualTimers implements Timers {
  nextId = 1;
  pending = new Map<number, () => void>();
  set(fn: () => void, _ms: number): unknown {
    const id = this.nextId++;
    this.pending.set(id, fn);
    return id;
  }
  clear(handle: unknown): void {
    if (typeof handle === 'number') this.pending.delete(handle);
  }
  flushAll(): void {
    const fns = [...this.pending.values()];
    this.pending.clear();
    for (const fn of fns) fn();
  }
}

function makePlatform(overrides: Partial<NonNullable<Platform['updates']>> = {}): {
  platform: Platform;
  check: ReturnType<typeof vi.fn>;
  download: ReturnType<typeof vi.fn>;
  apply: ReturnType<typeof vi.fn>;
} {
  const checkRes: AppUpdateCheckResult = {
    currentVersion: '0.1.0-m7.4',
    latestVersion: '0.1.0-m8',
    updateAvailable: true,
    tag: 'v0.1.0-m8',
    publishedAt: '2026-11-25T00:00:00Z',
    notesUrl: 'https://github.com/kumikit/kumikit/releases/tag/v0.1.0-m8',
    installKind: 'appimage',
    canAutoInstall: true,
    assetAvailable: true,
    assetSize: 1024,
  };
  const info = vi.fn().mockResolvedValue({
    currentVersion: '0.1.0-m7.4',
    target: 'x86_64-unknown-linux-gnu',
    installKind: 'appimage',
    canAutoInstall: true,
  });
  const check = vi.fn().mockResolvedValue(checkRes);
  const download = vi.fn().mockResolvedValue({
    version: '0.1.0-m8',
    path: '/tmp/updates/0.1.0-m8/Kumikit.AppImage',
    canAutoInstall: true,
    readyToInstall: true,
  });
  const apply = vi.fn().mockResolvedValue({
    applied: true,
    relaunching: true,
    message: 'Updated Kumikit to 0.1.0-m8.',
  });
  const platform = {
    kind: 'tauri',
    home: 'local',
    updates: {
      info: overrides.info ?? info,
      check: overrides.check ?? check,
      download: overrides.download ?? download,
      apply: overrides.apply ?? apply,
    },
  } as unknown as Platform;
  return { platform, check, download, apply };
}

function makeConn(activity: { activeTurns: number; readyQueued: number; idle: boolean }) {
  return {
    ready: true,
    call: vi.fn().mockResolvedValue(activity),
  } as unknown as CoreConnection & { call: ReturnType<typeof vi.fn> };
}

describe('UpdateManager', () => {
  it('defaults autoUpdate and autoSyncSsh to false and makes zero network requests on start', async () => {
    const parsed = parseSettings(null);
    expect(parsed.autoUpdate).toBe(false);
    expect(parsed.autoSyncSsh).toBe(false);

    const { platform, check, download, apply } = makePlatform();
    const settings = new SettingsStore(undefined);
    const timers = new ManualTimers();
    const mgr = new UpdateManager(platform, settings, () => makeConn({ activeTurns: 0, readyQueued: 0, idle: true }), {
      timers,
    });

    await mgr.start();
    expect(mgr.info?.currentVersion).toBe('0.1.0-m7.4');
    expect(check).not.toHaveBeenCalled();
    expect(download).not.toHaveBeenCalled();
    expect(apply).not.toHaveBeenCalled();
    expect(timers.pending.size).toBe(0);
  });

  it('waits while agents are active and automatically installs once all agents are idle', async () => {
    const { platform, check, download, apply } = makePlatform();
    const settings = new SettingsStore(undefined);
    const timers = new ManualTimers();
    let activity = { activeTurns: 1, readyQueued: 0, idle: false };
    const conn = {
      ready: true,
      call: vi.fn().mockImplementation(async () => activity),
    } as unknown as CoreConnection;

    const mgr = new UpdateManager(platform, settings, () => conn, { timers });
    await mgr.start();

    // Opt in: schedules background check.
    mgr.setAutoUpdate(true);
    expect(settings.value.autoUpdate).toBe(true);
    timers.flushAll();
    // Wait for async check + download + first idle check to settle.
    await new Promise((r) => setTimeout(r, 10));

    expect(check).toHaveBeenCalledTimes(1);
    expect(download).toHaveBeenCalledTimes(1);
    expect(apply).not.toHaveBeenCalled();
    expect(mgr.phase).toBe('waiting_idle');
    expect(mgr.activeTurns).toBe(1);

    // Still busy with a ready queued message: must still wait.
    activity = { activeTurns: 0, readyQueued: 2, idle: false };
    await mgr.tryApplyWhenIdle();
    expect(apply).not.toHaveBeenCalled();
    expect(mgr.phase).toBe('waiting_idle');
    expect(mgr.readyQueued).toBe(2);

    // All agents idle: applies with ifIdle=true.
    activity = { activeTurns: 0, readyQueued: 0, idle: true };
    const ok = await mgr.tryApplyWhenIdle();
    expect(ok).toBe(true);
    expect(apply).toHaveBeenCalledWith('0.1.0-m8', true);
    expect(mgr.phase).toBe('applied');
  });

  it('returns to waiting_idle if the core races and refuses shutdown as busy', async () => {
    const busyApply = vi.fn().mockRejectedValueOnce(new ConnectError('busy', 'turn started'));
    const { platform } = makePlatform({ apply: busyApply });
    const settings = new SettingsStore(undefined);
    settings.update({ autoUpdate: true });
    const timers = new ManualTimers();
    const conn = makeConn({ activeTurns: 0, readyQueued: 0, idle: true });

    const mgr = new UpdateManager(platform, settings, () => conn, { timers });
    await mgr.download({ autoApplyWhenIdle: true });
    expect(busyApply).toHaveBeenCalledWith('0.1.0-m8', true);
    expect(mgr.phase).toBe('waiting_idle');
    expect(timers.pending.size).toBeGreaterThan(0);
  });
});
