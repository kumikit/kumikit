// @vitest-environment jsdom
/**
 * Settings → Agents (06-client, Agents): which plugins are agents, the Agents page (installed
 * agents with their settings, registry agents installed through the grant dialog), and
 * agents left out of the plugin manager.
 */
import type { InstalledPlugin, Permissions, RegistryResult } from '@kumikit/protocol';
import { flushSync, mount, unmount } from 'svelte';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { Kernel } from '../src/app.svelte';
import { isAgent, isAgentResult, updatesFor } from '../src/plugins/agents';
import AgentsSettings from '../src/ui/AgentsSettings.svelte';
import PluginsView from '../src/ui/PluginsView.svelte';
import { FakeCore, ManualTimers, settle } from './fake-core';

const PERMS: Permissions = { spawn: ['agent'], network: [], files: 'none', secrets: [], tools: false };
const none = { providers: [], commands: [], settings: false };

function plugin(id: string, name: string, extra: Partial<InstalledPlugin> = {}): InstalledPlugin {
  return { id, name, version: '0.1.0', state: 'enabled', requested: PERMS, granted: PERMS, contributes: none, signed: true, source: 'registry', ...extra } as InstalledPlugin;
}

const claude = plugin('acme.claude', 'Claude', {
  contributes: { providers: ['acme.claude/claude'], commands: [], settings: true },
  providers: [{ id: 'acme.claude/claude', name: 'Claude' }],
});
const cursor = plugin('acme.cursor', 'Cursor', { state: 'disabled', granted: undefined, providers: [{ id: 'acme.cursor/cursor', name: 'Cursor' }] });
const git = plugin('acme.git', 'Git', { contributes: { providers: [], commands: ['git.status'], settings: true } });
// A plugin that only serves a provider backend is not an agent.
const backend = plugin('acme.acp', 'ACP agents', { source: 'bundled' });

const result = (id: string, name: string, kind: string[], starter = false): RegistryResult =>
  ({ id, name, kind, publisher: 'Acme', key: 'ed25519:k', registry: 'https://r.example/index.json', registryName: 'R', version: '0.2.0', requested: PERMS, starter }) as RegistryResult;

describe('agents', () => {
  it('are plugins that declare a provider, enabled or not', () => {
    expect([claude, cursor, git, backend].map(isAgent)).toEqual([true, true, false, false]);
    expect([result('a.x', 'X', ['provider']), result('a.y', 'Y', ['tool'])].map(isAgentResult)).toEqual([true, false]);
  });

  it('split updates between agents and other plugins', () => {
    const u = (plugin: string) => ({ plugin, from: '0.1.0', to: '0.2.0', registry: 'r', requested: PERMS, widens: false });
    const updates = [u('acme.claude'), u('acme.git')];
    expect(updatesFor(updates, [claude, git], true).map((x) => x.plugin)).toEqual(['acme.claude']);
    expect(updatesFor(updates, [claude, git], false).map((x) => x.plugin)).toEqual(['acme.git']);
  });
});

function setup() {
  const core = new FakeCore();
  let installed: InstalledPlugin[] = [claude, cursor, git, backend];
  core.handle('core.hello', () => ({
    protocol: '0.14',
    core: { version: '0', name: 'fake', os: 'linux' },
    plugins: [],
    capabilities: ['threads', 'plugins', 'registry', 'settings'],
  }));
  core.handle('plugin.list', () => ({ plugins: installed }));
  core.handle('registry.search', () => ({
    results: [result('acme.claude', 'Claude', ['provider']), result('acme.copilot', 'Copilot', ['provider'], true), result('acme.github', 'GitHub', ['tool'], true)],
  }));
  core.handle('plugin.install', (p) => {
    const id = (p.registry as { id: string }).id;
    const added = plugin(id, 'Copilot', { state: 'installed', granted: undefined, providers: [{ id: `${id}/copilot`, name: 'Copilot' }] });
    installed = [...installed, added];
    return { plugin: added };
  });
  core.handle('plugin.enable', (p) => {
    installed = installed.map((x) => (x.id === p.plugin ? { ...x, state: 'enabled' as const, granted: PERMS } : x));
    return { plugin: installed.find((x) => x.id === p.plugin) };
  });
  core.handle('plugin.disable', (p) => {
    installed = installed.map((x) => (x.id === p.plugin ? { ...x, state: 'disabled' as const } : x));
    return { plugin: installed.find((x) => x.id === p.plugin) };
  });
  core.handle('plugin.uninstall', (p) => {
    installed = installed.filter((x) => x.id !== p.plugin);
    return {};
  });
  core.handle('settings.schema', () => ({ schema: { type: 'object', properties: { model: { type: 'string', title: 'Default model' } } } }));
  core.handle('settings.get', () => ({ values: { model: 'opus' } }));
  core.handle('secret.list', () => ({ secrets: [] }));
  const kernel = new Kernel({ platform: { ...core.platform(), registry: undefined }, timers: new ManualTimers() });
  return { core, kernel };
}

const asked = (core: FakeCore, method: string) => core.requests.filter((r) => r.method === method);

function render(component: typeof AgentsSettings | typeof PluginsView, kernel: Kernel) {
  const target = document.createElement('div');
  document.body.append(target);
  const c = mount(component, { target, props: { kernel } });
  return {
    target,
    cleanup() {
      unmount(c);
      target.remove();
    },
  };
}

const names = (root: ParentNode, list: string) =>
  [...root.querySelectorAll(`[aria-label="${list}"] > li .title-col strong`)].map((e) => e.textContent?.trim());
const button = (root: ParentNode, text: string) =>
  [...root.querySelectorAll('button')].find((b) => b.textContent?.trim() === text) as HTMLButtonElement | undefined;

beforeEach(() => {
  vi.stubGlobal('fetch', async () => new Response('{}'));
  localStorage.clear();
});
afterEach(() => vi.unstubAllGlobals());

describe('the Agents page', () => {
  it('lists the installed agents, with settings only for an enabled one', async () => {
    const { kernel } = setup();
    await kernel.start();
    const { target, cleanup } = render(AgentsSettings, kernel);
    await settle();
    flushSync();
    expect(names(target, 'Installed agents')).toEqual(['Claude', 'Cursor']);
    const cards = [...target.querySelectorAll('[aria-label="Installed agents"] > li')];
    const summaries = (li: Element) => [...li.querySelectorAll('summary')].map((s) => s.textContent?.trim());
    expect(summaries(cards[0]!)).toEqual(['Permissions', 'Settings']);
    expect(summaries(cards[1]!)).toEqual(['Permissions']);
    expect(button(cards[1]!, 'Enable…')).toBeDefined();
    // The agent's settings come from the core's settings form.
    (cards[0]!.querySelector('details.settings') as HTMLDetailsElement).open = true;
    await settle();
    flushSync();
    expect(cards[0]!.textContent).toContain('Default model');
    cleanup();
  });

  it('asks the registries only when the user browses, and installs through the grant dialog', async () => {
    const { core, kernel } = setup();
    await kernel.start();
    const { target, cleanup } = render(AgentsSettings, kernel);
    await settle();
    flushSync();
    expect(asked(core, 'registry.search')).toEqual([]);
    button(target, 'Browse agents')!.click();
    await settle();
    flushSync();
    // Installed agents and non-agents are left out.
    expect(names(target, 'Available agents')).toEqual(['Copilot']);
    button(target, 'Install…')!.click();
    await settle();
    flushSync();
    expect(asked(core, 'plugin.enable')).toEqual([]);
    expect(document.body.textContent).toContain('Enable Copilot on');
    button(document.body, 'Grant and enable')!.click();
    await settle();
    flushSync();
    expect(asked(core, 'plugin.enable').map((r) => r.params)).toEqual([{ plugin: 'acme.copilot', grant: PERMS }]);
    expect(names(target, 'Installed agents')).toEqual(['Claude', 'Cursor', 'Copilot']);
    cleanup();
  });

  it('disables and removes an agent', async () => {
    const { core, kernel } = setup();
    await kernel.start();
    const { target, cleanup } = render(AgentsSettings, kernel);
    await settle();
    flushSync();
    button(target, 'Disable')!.click();
    await settle();
    flushSync();
    expect(asked(core, 'plugin.disable').map((r) => r.params)).toEqual([{ plugin: 'acme.claude' }]);
    button(target, 'Remove…')!.click();
    flushSync();
    button(document.body, 'Remove, keep data')!.click();
    await settle();
    flushSync();
    expect(asked(core, 'plugin.uninstall').map((r) => r.params)).toEqual([{ plugin: 'acme.claude', deleteData: false }]);
    expect(names(target, 'Installed agents')).toEqual(['Cursor']);
    cleanup();
  });
});

describe('the plugin manager', () => {
  it('leaves agents out of the core plugins and the registry results', async () => {
    const { kernel } = setup();
    await kernel.start();
    const { target, cleanup } = render(PluginsView, kernel);
    await settle();
    flushSync();
    const core = target.querySelector('[aria-labelledby="kk-core-plugins"]')!;
    expect([...core.querySelectorAll('.list > li strong')].map((e) => e.textContent?.trim())).toEqual(['Git', 'ACP agents']);
    expect(core.textContent).toContain('2 agents are installed here too.');
    await kernel.registries.search('');
    flushSync();
    expect(names(target, 'Registry results')).toEqual(['GitHub']);
    cleanup();
  });

  it('keeps agents in the starter pack', async () => {
    const { kernel } = setup();
    await kernel.start();
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(PluginsView, { target, props: { kernel, starter: true } });
    await kernel.registries.search('');
    await settle();
    flushSync();
    expect(names(target, 'Registry results')).toEqual(['Copilot', 'GitHub']);
    unmount(c);
    target.remove();
  });
});
