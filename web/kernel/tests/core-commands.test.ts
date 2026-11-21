// @vitest-environment jsdom
/** Core plugins' commands in the palette and their results (07-ui-plugins, Level 1). */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { Kernel } from '../src/app.svelte';
import { coreWhen } from '../src/commands/core.svelte';
import { FakeCore, ManualTimers, platformOf, settle } from './fake-core';

function setup() {
  const local = new FakeCore();
  const hello = local.handlers.get('core.hello')!;
  local.handle('core.hello', (p) => {
    const r = hello(p) as { capabilities: string[]; plugins: unknown[] };
    return { ...r, capabilities: [...r.capabilities, 'commands'], plugins: [{ id: 'acme.tool', version: '0.1.0', contributes: { providers: [], commands: ['greet', 'scan'], settings: false } }] };
  });
  local.handle('workspace.list', () => ({ workspaces: [{ id: 'w1', name: 'proj', root: '/p' }] }));
  local.handle('command.list', () => ({
    commands: [
      { plugin: 'acme.tool', id: 'greet', title: 'Greet', scope: 'core' },
      { plugin: 'acme.tool', id: 'scan', title: 'Scan the project', scope: 'workspace' },
    ],
  }));
  local.handle('command.run', (p) =>
    p.command === 'greet'
      ? { message: 'hello', view: { model: 'form', schema: { type: 'object', properties: { name: { type: 'string' } } }, values: {}, submit: 'greet' } }
      : { message: `scanned ${String(p.workspace)}` },
  );
  const kernel = new Kernel({ platform: platformOf({ local }), timers: new ManualTimers() });
  return { local, kernel };
}

beforeEach(() => {
  vi.stubGlobal('fetch', async () => new Response('{}'));
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe('core commands', () => {
  it('get a when clause that needs a tab for workspace commands', () => {
    expect(coreWhen({ plugin: 'a.b', id: 'x', title: 'X', scope: 'workspace', when: 'thread.open' })).toBe('tab.open && thread.open');
    expect(coreWhen({ plugin: 'a.b', id: 'x', title: 'X', scope: 'core' })).toBe('');
  });

  it('are listed from the active core, run there and show their result', async () => {
    const { local, kernel } = setup();
    await kernel.start();
    await settle();
    await settle();
    const titles = () => kernel.commands.search('', kernel.when, kernel.commandAllowed).map((c) => c.id);
    expect(titles()).toContain('core:acme.tool:greet');
    // A workspace command waits for a tab.
    expect(titles()).not.toContain('core:acme.tool:scan');

    await kernel.commands.run('core:acme.tool:greet');
    expect(local.requests.at(-1)).toEqual({ method: 'command.run', params: { plugin: 'acme.tool', command: 'greet' } });
    expect(kernel.toasts.at(-1)?.message).toBe('hello');
    expect(kernel.main.kind).toBe('core-view');

    kernel.openTab('local', 'w1');
    await settle();
    expect(titles()).toContain('core:acme.tool:scan');
    await kernel.commands.run('core:acme.tool:scan');
    expect(local.requests.at(-1)?.params).toEqual({ plugin: 'acme.tool', command: 'scan', workspace: 'w1' });
    expect(kernel.toasts.at(-1)?.message).toBe('scanned w1');
  });
});
