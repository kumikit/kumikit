// @vitest-environment jsdom
/** Plugins from registries in the UI (06-client, Plugin manager; 10-distribution): nothing
 * is fetched until the user asks, wider updates grant only what was shown, lockfiles. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { Kernel } from '../src/app.svelte';
import { parseLock, registryOf, withClientFlags } from '../src/plugins/lockfile';
import { importPlan } from '../src/plugins/registry.svelte';
import type { ClientPluginEntry, Platform } from '../src/platform/index';
import { FakeCore, ManualTimers, settle } from './fake-core';

// The exact text kumikit_protocol::lockfile::render writes.
const LOCK = `# Written by Kumikit. Export it to reproduce these plugins on another core.
format = 1

[[plugin]]
id = "a.one"
version = "0.1.0"
sha256 = "cd"
source = "registry+https://r.example/index.json"
key = "ed25519:x"
state = "enabled"

[plugin.granted]
spawn = ["agent"]
network = []
files = "read_workspace"
secrets = []
tools = false
client = ["threads.send"]
workspaces = "linked"
schedules = true

[[plugin]]
id = "b.two"
version = "1.0.0"
sha256 = "ab"
source = "file"
state = "installed"
`;

describe('lockfiles', () => {
  it('read what Kumikit writes', () => {
    const lock = parseLock(LOCK);
    expect(lock.plugins.map((p) => p.id)).toEqual(['a.one', 'b.two']);
    expect(lock.plugins[0]!.granted).toEqual({
      spawn: ['agent'],
      network: [],
      files: 'read_workspace',
      secrets: [],
      tools: false,
      client: ['threads.send'],
      workspaces: 'linked',
      schedules: true,
    });
    expect(registryOf(lock.plugins[0]!)).toBe('https://r.example/index.json');
    expect(registryOf(lock.plugins[1]!)).toBeUndefined();
  });

  it('refuse anything else, with a line number', () => {
    expect(() => parseLock('format = 2\n')).toThrow('format 2');
    expect(() => parseLock('format = 1\n[surprise]\n')).toThrow('line 2');
    expect(() => parseLock('format = 1\n[[plugin]]\nid = { inline = 1 }\n')).toThrow('line 3');
    expect(() => parseLock('nothing here')).toThrow();
    expect(() => parseLock('format = 1\n[[plugin]]\nid = "x"\n')).toThrow('no version');
  });

  it('get the client flags on export', () => {
    const out = withClientFlags(LOCK, (id) => ({ enabled: id === 'a.one', trusted: false }));
    const lock = parseLock(out);
    expect(lock.plugins.map((p) => [p.id, p.enabled, p.trusted])).toEqual([
      ['a.one', true, false],
      ['b.two', false, false],
    ]);
  });

  it('plan an import without doing anything', () => {
    const lock = parseLock(LOCK.replace('source = "file"', 'source = "registry+https://other.example/index.json"'));
    const plan = importPlan(lock, new Map(), ['https://r.example/index.json']);
    expect(plan.map((i) => i.action)).toEqual(['install', 'unknown-registry']);
    expect(importPlan(lock, new Map([['a.one', '0.1.0'], ['b.two', '0.9.0']]), []).map((i) => i.action)).toEqual(['installed', 'other-version']);
    expect(importPlan(parseLock(LOCK), new Map(), []).map((i) => i.action)).toEqual(['unknown-registry', 'not-from-registry']);
  });
});

const PERMS = { spawn: ['agent'], network: [], files: 'none', secrets: [], tools: false };

function setup() {
  const core = new FakeCore();
  core.handle('core.hello', () => ({
    protocol: '0.6',
    core: { version: '0', name: 'fake', os: 'linux' },
    plugins: [],
    capabilities: ['threads', 'plugins', 'registry'],
  }));
  core.handle('plugin.list', () => ({ plugins: [] }));
  core.handle('registry.list', () => ({ registries: [{ url: 'https://r.example/index.json' }] }));
  core.handle('registry.search', () => ({
    results: [
      { id: 'acme.probe', name: 'Probe', kind: ['provider'], publisher: 'Acme', key: 'ed25519:k', registry: 'https://r.example/index.json', registryName: 'R', version: '0.1.0', requested: PERMS, starter: true },
      { id: 'acme.other', name: 'Other', kind: ['provider'], publisher: 'Acme', key: 'ed25519:k', registry: 'https://r.example/index.json', registryName: 'R', version: '0.1.0', requested: PERMS },
    ],
  }));
  core.handle('plugin.install', (p) => ({ plugin: { id: (p.registry as { id: string }).id, name: 'Probe', version: '0.1.0', state: 'installed', requested: PERMS, contributes: { providers: [], commands: [], settings: false }, signed: true, publisher: { key: 'ed25519:k', registry: 'https://r.example/index.json' }, source: 'registry' } }));
  core.handle('plugin.updates', () => ({
    updates: [
      { plugin: 'acme.wide', from: '0.1.0', to: '0.2.0', registry: 'https://r.example/index.json', requested: { ...PERMS, secrets: ['x'] }, widens: true },
      { plugin: 'acme.same', from: '0.1.0', to: '0.1.1', registry: 'https://r.example/index.json', requested: PERMS, widens: false },
    ],
  }));
  core.handle('plugin.update', (p) => ({ plugin: { id: p.plugin, name: String(p.plugin), version: p.version, state: 'enabled', requested: PERMS, contributes: { providers: [], commands: [], settings: false }, signed: true } }));
  const calls: string[] = [];
  const entry: ClientPluginEntry = { id: 'acme.night', version: '1.0.0', source: 'installed', manifest: { id: 'acme.night', name: 'Night', version: '1.0.0', kind: ['theme'], api: '^0.1', theme: { variants: [] } } };
  const platform: Platform = {
    ...core.platform(),
    registry: {
      search: async () => {
        calls.push('search');
        return [{ id: 'acme.night', name: 'Night', kind: ['theme'], publisher: 'Acme', key: 'ed25519:k', registry: 'https://r.example/index.json', registryName: 'R', version: '1.0.0', ui: {}, starter: false }];
      },
      install: async (r) => {
        calls.push(`install ${r.id} ${r.registry}`);
        return entry;
      },
      updates: async () => {
        calls.push('updates');
        return [];
      },
      update: async () => entry,
      registries: async () => {
        calls.push('registries');
        return ['https://r.example/index.json'];
      },
      setRegistries: async () => {},
      lock: async () => LOCK,
    },
  };
  const kernel = new Kernel({ platform, timers: new ManualTimers() });
  return { core, kernel, calls };
}

beforeEach(() => {
  vi.stubGlobal('fetch', async () => new Response('{}'));
  localStorage.clear();
});
afterEach(() => vi.unstubAllGlobals());

const asked = (core: FakeCore, ...methods: string[]) => core.requests.filter((r) => methods.includes(r.method));

describe('registries in the plugin manager', () => {
  it('ask no registry until the user does', async () => {
    const { core, kernel, calls } = setup();
    await kernel.start();
    await settle();
    expect(asked(core, 'registry.search', 'plugin.updates', 'plugin.install')).toEqual([]);
    expect(calls.filter((c) => c !== 'registries')).toEqual([]);
    expect(kernel.settings.value.welcomeDone).toBe(false);
  });

  it('the starter pack is answered once and searched only when chosen', async () => {
    const { core, kernel, calls } = setup();
    await kernel.start();
    await settle();
    kernel.dismissWelcome();
    expect(kernel.settings.value.welcomeDone).toBe(true);
    expect(asked(core, 'registry.search')).toEqual([]);
    kernel.openStarterPack();
    await settle();
    expect(kernel.main).toEqual({ kind: 'plugins', starter: true });
    expect(asked(core, 'registry.search')).toHaveLength(1);
    expect(calls).toContain('search');
    expect(kernel.registries.core?.filter((r) => r.starter).map((r) => r.id)).toEqual(['acme.probe']);
  });

  it('install from the registry the result came from, granting nothing', async () => {
    const { core, kernel, calls } = setup();
    await kernel.start();
    await settle();
    await kernel.registries.search('probe');
    const p = await kernel.registries.installCore(kernel.registries.core![0]!);
    expect(asked(core, 'plugin.install').map((r) => r.params)).toEqual([{ registry: { id: 'acme.probe', registry: 'https://r.example/index.json' } }]);
    expect(p?.publisher?.key).toBe('ed25519:k');
    expect(asked(core, 'plugin.enable')).toEqual([]);
    await kernel.registries.installClient(kernel.registries.client![0]!);
    expect(calls).toContain('install acme.night https://r.example/index.json');
  });

  it('a wider update grants exactly the new permissions; a same one grants nothing', async () => {
    const { core, kernel } = setup();
    await kernel.start();
    await settle();
    await kernel.registries.checkUpdates();
    expect(asked(core, 'plugin.update')).toEqual([]);
    const [wide, same] = kernel.registries.coreUpdates!;
    await kernel.registries.updateCore(wide!);
    await kernel.registries.updateCore(same!);
    expect(asked(core, 'plugin.update').map((r) => r.params)).toEqual([
      { plugin: 'acme.wide', version: '0.2.0', grant: { ...PERMS, secrets: ['x'] } },
      { plugin: 'acme.same', version: '0.1.1' },
    ]);
    expect(kernel.registries.coreUpdates).toEqual([]);
  });

  it('export the app lockfile with trust, and plan an import against the core', async () => {
    const { kernel } = setup();
    await kernel.start();
    await settle();
    kernel.settings.update({ trustedPlugins: ['a.one'] });
    const lock = await kernel.registries.clientLock();
    expect(parseLock(lock!).plugins[0]).toMatchObject({ id: 'a.one', enabled: true, trusted: true });
    const plan = await kernel.registries.corePlan(LOCK);
    expect(plan.map((i) => i.action)).toEqual(['install', 'not-from-registry']);
  });
});
