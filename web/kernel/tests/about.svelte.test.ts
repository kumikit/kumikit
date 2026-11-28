// @vitest-environment jsdom
/**
 * Settings → About (06-client, About): the version sentence stays one piece, and each
 * connected core's resource use comes from `core.stats`.
 */
import { flushSync, mount, unmount } from 'svelte';
import { describe, expect, it } from 'vitest';

import type { Kernel } from '../src/app.svelte';
import { formatBytes, formatUptime } from '../src/ui/about';
import AboutSettings from '../src/ui/AboutSettings.svelte';

describe('about formatting', () => {
  it('formats bytes and uptime', () => {
    expect([0, 1, 2048, 16_384_000, 3 * 1024 ** 3].map(formatBytes)).toEqual(['0 KB', '1 KB', '2 KB', '15.6 MB', '3.0 GB']);
    expect([45, 720, 3 * 3600 + 300, 2 * 86_400 + 4 * 3600].map(formatUptime)).toEqual(['45 s', '12 min', '3 h 5 min', '2 d 4 h']);
  });
});

function render(connections: unknown[]) {
  const kernel = {
    updates: {
      supported: true,
      phase: 'up_to_date',
      checkResult: { latestVersion: '0.1.0-m9.2' },
      info: { currentVersion: '0.1.0-m9.2', installKind: 'appimage' },
      hasAttention: false,
    },
    connections,
    settings: { value: { autoUpdate: false, autoSyncSsh: false } },
  } as unknown as Kernel;
  const target = document.createElement('div');
  document.body.append(target);
  const c = mount(AboutSettings, { target, props: { kernel } });
  return {
    target,
    cleanup() {
      unmount(c);
      target.remove();
    },
  };
}

const settle = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
  flushSync();
};

describe('the About page', () => {
  it('keeps the up-to-date sentence in one element, so the brackets stay with the version', async () => {
    const { target, cleanup } = render([]);
    await settle();
    const status = target.querySelector('.status.ok')!;
    // `.status` is a flex row: loose text runs next to <code> become separate flex items
    // and are spread apart. All of the sentence must sit in one child element.
    const loose = [...status.childNodes].filter((n) => n.nodeType === Node.TEXT_NODE && n.textContent!.trim() !== '');
    expect(loose).toEqual([]);
    expect(status.children).toHaveLength(1);
    expect(status.children[0]!.textContent?.replace(/\s+/g, ' ').trim()).toBe('You are on the latest signed release (v0.1.0-m9.2).');
    cleanup();
  });

  it('shows what each connected core uses, and a core that cannot say', async () => {
    const local = {
      id: 'local',
      label: 'This computer',
      ready: true,
      hello: { core: { version: '0.1.0', name: 'box' } },
      call: async () => ({ memoryBytes: 16_384_000, databaseBytes: 4_718_592, activeTurns: 2, processes: 3, uptimeSeconds: 3725 }),
    };
    const old = { id: 'old', label: 'Old box', ready: true, call: async () => Promise.reject(new Error('unknown method core.stats')) };
    const down = { id: 'down', label: 'Offline', ready: false, call: async () => ({}) };
    const { target, cleanup } = render([local, old, down]);
    await settle();
    const card = target.querySelector('[aria-label="Resource usage of This computer"]')!;
    const values = Object.fromEntries([...card.querySelectorAll('dl > div')].map((d) => [d.querySelector('dt')!.textContent, d.querySelector('dd')!.textContent]));
    expect(values).toEqual({ 'Core memory': '15.6 MB', 'Active agents': '2 running', Processes: '3', Database: '4.5 MB', Uptime: '1 h 2 min' });
    expect(target.querySelector('[aria-label="Resource usage of Old box"]')?.textContent).toContain('Cannot read: unknown method core.stats');
    expect(target.querySelector('[aria-label="Resource usage of Offline"]')).toBeNull();
    cleanup();
  });
});
