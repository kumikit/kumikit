// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';

import { Kernel } from '../src/app.svelte';
import type { ClientPluginEntry, Platform } from '../src/platform/index';
import { FakeCore, settle } from './fake-core';

type Files = Record<string, string>;

const theme = (id: string, variants: [string, 'light' | 'dark'][]): ClientPluginEntry['manifest'] => ({
  id,
  name: id === 'kumikit.base' ? 'Base' : 'Night',
  version: '1.0.0',
  kind: ['theme'],
  theme: { variants: variants.map(([v, appearance]) => ({ id: v, appearance, tokens: `${v}.json` })) },
});
const ui = (id: string): ClientPluginEntry['manifest'] => ({ id, name: id, version: '1.0.0', kind: ['ui'], ui: { entry: 'index.js' } });

function setup(entries: ClientPluginEntry[], files: Files, opts: { safe?: boolean; bundled?: object } = {}) {
  const url = (id: string, path: string) => `kk-test://${id}/${path}`;
  vi.stubGlobal('fetch', async (u: string) => {
    if (u === './bundled.json') return new Response(JSON.stringify(opts.bundled ?? { fallbackTheme: 'kumikit.base' }));
    const body = files[u];
    return body === undefined ? new Response('missing', { status: 404 }) : new Response(body);
  });
  const platform: Platform = {
    ...new FakeCore().platform(),
    pluginUrl: (id, path) => {
      const code = files[url(id, path)];
      return code !== undefined && path.endsWith('.js') ? `data:text/javascript,${encodeURIComponent(code)}` : url(id, path);
    },
    clientPlugins: { list: async () => entries, install: async () => { throw new Error('no'); }, remove: async () => {} },
    safeMode: opts.safe ?? false,
  };
  return new Kernel({ platform, systemDark: () => true });
}

const entry = (id: string, manifest: ClientPluginEntry['manifest'], source: 'bundled' | 'installed' = 'bundled'): ClientPluginEntry => ({
  id,
  version: '1.0.0',
  source,
  manifest,
});

const PLUGIN = `export default {
  activate(ctx) {
    ctx.slots.register('statusbar.item', { id: 'hello', mount(el) { el.textContent = 'hi from ' + ctx.id; } });
    ctx.commands.register({ id: 'hello.say', title: 'Say hello' }, () => 'hello');
  },
};`;

afterEach(() => vi.unstubAllGlobals());

describe('client plugins', () => {
  it('load themes with the fallback from bundled.json and report bad tokens', async () => {
    const k = setup(
      [entry('kumikit.base', theme('kumikit.base', [['light', 'light'], ['dark', 'dark']])), entry('community.night', theme('community.night', [['night', 'dark']]))],
      {
        'kk-test://kumikit.base/light.json': '{"accent":"#00f","surface.panel":"#fff"}',
        'kk-test://kumikit.base/dark.json': '{"accent":"#88f","surface.panel":"#000"}',
        'kk-test://community.night/night.json': '{"accent":"#f0f","text.primary":"red; x"}',
      },
    );
    await k.clientPlugins.load();
    expect(k.theme.shown).toBe('kumikit.base:dark');
    k.selectTheme('community.night:night');
    expect(k.theme.tokens).toEqual({ accent: '#f0f', 'surface.panel': '#000' });
    expect(k.toasts.some((t) => t.message.includes('text.primary: not a valid color'))).toBe(true);
    expect(k.commands.commands.map((c) => c.id)).toContain('theme.select:community.night:night');
  });

  it('activate UI plugins, and remove what they added when disabled', async () => {
    const k = setup([entry('community.hello', ui('community.hello'))], { 'kk-test://community.hello/index.js': PLUGIN });
    await k.clientPlugins.load();
    expect(k.slots.in('statusbar.item').map((e) => e.plugin)).toEqual(['community.hello']);
    expect(await k.commands.run('hello.say')).toBe('hello');
    await k.clientPlugins.setEnabled('community.hello', false);
    expect(k.slots.in('statusbar.item')).toEqual([]);
    expect(k.commands.commands.some((c) => c.id === 'hello.say')).toBe(false);
    expect(k.settings.value.disabledPlugins).toEqual(['community.hello']);
  });

  it('report a plugin that cannot start', async () => {
    const k = setup([entry('community.bad', ui('community.bad'))], {
      'kk-test://community.bad/index.js': 'export default { activate() { throw new Error("boom"); } };',
    });
    await k.clientPlugins.load();
    expect(k.clientPlugins.plugins[0]!.error).toBe('boom');
    expect(k.toasts.at(-1)?.message).toContain('could not start: boom');
  });

  it('wait for trust before running third-party code', async () => {
    const k = setup([entry('community.hello', ui('community.hello'), 'installed')], { 'kk-test://community.hello/index.js': PLUGIN });
    await k.clientPlugins.load();
    expect(k.clientPlugins.plugins[0]).toMatchObject({ needsTrust: true, enabled: false });
    expect(k.slots.in('statusbar.item')).toEqual([]);
    await k.clientPlugins.trust('community.hello');
    expect(k.slots.in('statusbar.item')).toHaveLength(1);
  });

  it('load no UI plugin in safe mode, but still load themes', async () => {
    const k = setup(
      [entry('kumikit.base', theme('kumikit.base', [['dark', 'dark']])), entry('community.hello', ui('community.hello'))],
      { 'kk-test://kumikit.base/dark.json': '{"accent":"#88f"}', 'kk-test://community.hello/index.js': PLUGIN },
      { safe: true },
    );
    await k.clientPlugins.load();
    await settle();
    expect(k.slots.entries).toEqual([]);
    expect(k.theme.tokens.accent).toBe('#88f');
  });
});
