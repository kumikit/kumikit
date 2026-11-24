// @vitest-environment jsdom
import type { PluginContext, RendererProps, ThreadView } from '@kumikit/ui-sdk';
import { flushSync, mount, unmount } from 'svelte';
import { describe, expect, it } from 'vitest';

import Approval from '../src/Approval.svelte';
import Chat from '../src/Chat.svelte';
import Handoff from '../src/Handoff.svelte';
import Steer from '../src/Steer.svelte';

const address = { connection: 'local', workspace: 'w', thread: 't' };

function fakeCtx(view: ThreadView) {
  const ran: [string, unknown][] = [];
  let push: (v: ThreadView) => void = () => {};
  const ctx = {
    threads: {
      watch: (_a: unknown, fn: (v: ThreadView) => void) => {
        push = fn;
        fn(view);
        return () => {};
      },
    },
    commands: { run: async (c: string, a?: unknown) => void ran.push([c, a]) },
    ui: {
      markdown: (el: HTMLElement, t: string) => (el.textContent = t),
      renderItem: (el: HTMLElement, p: RendererProps) => {
        el.textContent = p.item.kind === 'message' ? p.item.text : p.item.kind;
      },
    },
  } as unknown as PluginContext;
  return { ctx, ran, push: (v: ThreadView) => push(v) };
}

const base: ThreadView = { address, status: 'idle', seq: 0, turns: [], pendingApprovals: [], queued: [], queue: [], live: true };
const entry = (id: string, text: string) => ({ id, turn: `u-${id}`, input: { text } });
const key = (k: string, extra: KeyboardEventInit = {}) => new KeyboardEvent('keydown', { key: k, bubbles: true, cancelable: true, ...extra });

describe('the queue panel', () => {
  function queued(view: Partial<ThreadView>) {
    const { ctx, ran, push } = fakeCtx(base);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();
    push({ ...base, ...view });
    flushSync();
    const panel = target.querySelector<HTMLElement>('section[aria-label="Queued messages"]');
    const button = (label: string) => target.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`)!;
    return { target, panel, ran, button, done: () => unmount(c) };
  }

  it('is hidden with an empty queue', () => {
    const { panel, done } = queued({});
    expect(panel).toBeNull();
    done();
  });

  it('lists entries and sends, removes and resumes through the core', async () => {
    const { panel, ran, button, target, done } = queued({
      status: 'running',
      queue: [entry('a', 'first line\nmore'), entry('b', 'second')],
      queueHold: 'stopped',
    });
    expect(panel!.textContent).toContain('first line');
    expect(panel!.textContent).not.toContain('more');
    expect(panel!.textContent).toContain('Paused after you stopped the run');
    button('Send now: second').click();
    button('Remove queued message 1').click();
    [...target.querySelectorAll('button')].find((b) => b.textContent?.trim() === 'Resume after this run')!.click();
    await Promise.resolve();
    expect(ran).toEqual([
      ['thread.queue.sendNow', { address, entry: 'b' }],
      ['thread.queue.remove', { address, entry: 'a' }],
      ['thread.queue.resume', { address }],
    ]);
    done();
  });

  it('edits in place: Enter saves, IME and Escape do not', async () => {
    const { ran, button, target, done } = queued({ status: 'running', queue: [entry('a', 'old')] });
    button('Edit queued message 1').click();
    flushSync();
    const box = target.querySelector<HTMLTextAreaElement>('textarea[id="queue-edit-a"]')!;
    box.value = 'new text';
    box.dispatchEvent(new Event('input', { bubbles: true }));
    box.dispatchEvent(key('Enter', { isComposing: true }));
    expect(ran).toEqual([]);
    box.dispatchEvent(key('Enter'));
    flushSync();
    await Promise.resolve();
    expect(ran).toEqual([['thread.queue.update', { address, entry: 'a', text: 'new text' }]]);

    button('Edit queued message 1').click();
    flushSync();
    target.querySelector<HTMLTextAreaElement>('textarea[id="queue-edit-a"]')!.dispatchEvent(key('Escape'));
    flushSync();
    expect(target.querySelector('textarea[id="queue-edit-a"]')).toBeNull();
    expect(ran).toHaveLength(1);
    done();
  });

  it('moves an entry with Alt+Up and Alt+Down', async () => {
    const { ran, target, done } = queued({ queue: [entry('a', 'one'), entry('b', 'two'), entry('c', 'three')] });
    const row = (id: string) => target.querySelector<HTMLElement>(`li[data-entry="${id}"]`)!;
    row('a').dispatchEvent(key('ArrowDown', { altKey: true }));
    row('c').dispatchEvent(key('ArrowUp', { altKey: true }));
    row('c').dispatchEvent(key('ArrowDown', { altKey: true }));
    row('b').dispatchEvent(key('ArrowDown'));
    await Promise.resolve();
    expect(ran).toEqual([
      ['thread.queue.move', { address, entry: 'a', before: 'c' }],
      ['thread.queue.move', { address, entry: 'c', before: 'b' }],
    ]);
    done();
  });

  it('shows the Not steered badge and offers Steer when the model can be steered', async () => {
    const viewWithModel: ThreadView = {
      ...base,
      status: 'running',
      thread: {
        id: 't',
        workspace: 'w',
        provider: 'test/p',
        model: 'steerable',
        permissionMode: 'standard',
        status: 'running',
        created: '2026-11-24T00:00:00Z',
        updated: '2026-11-24T00:00:00Z',
      },
      queue: [entry('a', 'change course')],
      notSteered: ['a'],
    };
    const connection = {
      id: 'local',
      ready: true,
      call: async () => ({}),
      subscribe: () => () => {},
      models: async () => [{ id: 'steerable', label: 'Steerable', capabilities: ['steer'] }],
    };
    const { ctx, ran } = fakeCtx(viewWithModel);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address, connection: connection as never } });
    flushSync();
    await Promise.resolve();
    flushSync();

    const panel = target.querySelector<HTMLElement>('section[aria-label="Queued messages"]')!;
    expect(panel.textContent).toContain('Not steered');
    const steerBtn = target.querySelector<HTMLButtonElement>('button[aria-label="Steer: change course"]')!;
    expect(steerBtn).not.toBeNull();
    steerBtn.click();
    await Promise.resolve();
    expect(ran).toEqual([['thread.queue.steer', { address, entry: 'a' }]]);

    // Ctrl+Enter steers while running, Enter queues.
    const box = target.querySelector('textarea')!;
    box.value = 'go left';
    box.dispatchEvent(new Event('input', { bubbles: true }));
    box.dispatchEvent(key('Enter', { ctrlKey: true }));
    flushSync();
    expect(ran.at(-1)).toEqual(['thread.steer', { address, text: 'go left' }]);
    expect(box.value).toBe('');

    box.value = 'for later';
    box.dispatchEvent(new Event('input', { bubbles: true }));
    box.dispatchEvent(key('Enter'));
    flushSync();
    expect(ran.at(-1)).toEqual(['thread.send', { address, text: 'for later' }]);

    unmount(c);
  });
});

describe('the chat view', () => {
  it('sends on Enter, not during IME composition, and keeps Shift+Enter for new lines', () => {
    const { ctx, ran } = fakeCtx(base);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();
    const box = target.querySelector('textarea')!;
    box.value = 'こんにちは';
    box.dispatchEvent(new Event('input', { bubbles: true }));
    box.dispatchEvent(key('Enter', { isComposing: true }));
    box.dispatchEvent(key('Enter', { shiftKey: true }));
    expect(ran).toEqual([]);
    box.dispatchEvent(key('Enter'));
    flushSync();
    expect(ran).toEqual([['thread.send', { address, text: 'こんにちは' }]]);
    expect(box.value).toBe('');
    unmount(c);
  });

  it('shows turns and their items, and cancels on a second Escape while busy', () => {
    const { ctx, ran, push } = fakeCtx(base);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();
    push({
      ...base,
      status: 'running',
      turns: [{ id: 'u1', input: 'fix it', started: 't', items: [{ kind: 'message', text: 'On it', done: false }] }],
      queued: ['and then this'],
      queue: [entry('q1', 'and then this')],
    });
    flushSync();
    expect(target.textContent).toContain('fix it');
    expect(target.textContent).toContain('On it');
    expect(target.textContent).toContain('and then this');
    const box = target.querySelector('textarea')!;
    box.dispatchEvent(key('Escape'));
    flushSync();
    expect(target.textContent).toContain('Press Escape again');
    expect(ran).toEqual([]);
    box.dispatchEvent(key('Escape'));
    expect(ran).toEqual([['thread.cancel', undefined]]);
    unmount(c);
  });

  it('displays the selected model below the composer and changes permission mode via popover', () => {
    const viewWithThread: ThreadView = {
      ...base,
      thread: {
        id: 't',
        workspace: 'w',
        provider: 'test/p',
        model: 'opus-5-5',
        permissionMode: 'standard',
        status: 'idle',
        created: '2026-11-22T00:00:00Z',
        updated: '2026-11-22T00:00:00Z',
      },
    };
    const { ctx, ran } = fakeCtx(viewWithThread);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();

    expect(target.textContent).toContain('opus-5-5');
    const permBtn = target.querySelector<HTMLButtonElement>('button[aria-label^="Permissions:"]');
    expect(permBtn).not.toBeNull();
    permBtn!.click();
    flushSync();

    const fullAccess = [...target.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]')].find((b) =>
      b.textContent?.includes('Full access'),
    );
    expect(fullAccess).toBeDefined();
    fullAccess!.click();
    flushSync();

    expect(ran).toEqual([['thread.update', { address, permissionMode: 'full_access' }]]);
    unmount(c);
  });

  it('loads workspace git status and workspace list once on mount without looping', async () => {
    const calls: string[] = [];
    const connection = {
      id: 'local',
      ready: true,
      call: async (method: string) => {
        calls.push(method);
        if (method === 'workspace.git.status') {
          return {
            isRepo: true,
            branch: 'main',
            detached: false,
            isWorktree: false,
            repoRoot: '/tmp/proj',
            defaultBranch: 'main',
            branches: [{ name: 'main', current: true }],
          };
        }
        if (method === 'workspace.list') {
          return { workspaces: [{ id: 'w', name: 'proj', root: '/tmp/proj' }] };
        }
        return {};
      },
      subscribe: () => () => {},
      models: async () => [],
    };
    const { ctx } = fakeCtx(base);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address, connection: connection as never } });
    flushSync();
    // Flush several microtask turns to prove no reactive re-trigger loop occurs.
    for (let i = 0; i < 6; i++) {
      await Promise.resolve();
      flushSync();
    }
    expect(calls.filter((m) => m === 'workspace.git.status')).toHaveLength(1);
    expect(calls.filter((m) => m === 'workspace.list')).toHaveLength(1);
    unmount(c);
  });
});

describe('the approval renderer', () => {
  const approval = { id: 'a1', thread: 't', turn: 'u1', kind: 'tool', summary: 'Edit a.txt', status: 'pending' as const };

  it('answers through the kernel command and shows the result once resolved', () => {
    const ran: unknown[] = [];
    const target = document.createElement('div');
    const c = mount(Approval, { target, props: { item: { kind: 'approval', approval }, run: async (cmd: string, a?: unknown) => void ran.push([cmd, a]) } });
    flushSync();
    const buttons = [...target.querySelectorAll('button')].map((b) => b.textContent);
    expect(buttons).toEqual(['Allow', 'Always allow', 'Deny']);
    target.querySelector('button')!.click();
    expect(ran).toEqual([['approval.respond', { approval: 'a1', decision: 'allow' }]]);
    unmount(c);
    const done = mount(Approval, { target, props: { item: { kind: 'approval', approval, resolution: 'deny' }, run: async () => {} } });
    flushSync();
    expect(target.textContent).toContain('Denied');
    expect(target.querySelectorAll('button')).toHaveLength(0);
    unmount(done);
  });
});

describe('the steer renderer', () => {
  it('renders a steered message with its badge', () => {
    const target = document.createElement('div');
    const c = mount(Steer, { target, props: { text: 'use the existing helper' } });
    flushSync();
    expect(target.textContent).toContain('Steered');
    expect(target.textContent).toContain('use the existing helper');
    unmount(c);
  });
});

describe('the handoff renderer and provider switch', () => {
  it('renders a collapsible handoff with the exact carried-forward context', () => {
    const target = document.createElement('div');
    const c = mount(Handoff, {
      target,
      props: {
        item: {
          kind: 'handoff',
          from: { provider: 'p.a/a', model: 'm1' },
          to: { provider: 'p.b/b', model: 'm2' },
          messages: 4,
          truncated: true,
          rendered: '[Conversation handoff from p.a/a (m1)]\nUser: hi',
        },
      },
    });
    flushSync();
    expect(target.textContent).toContain('Conversation carried forward from p.a/a (m1)');
    expect(target.textContent).toContain('4 messages (trimmed)');
    expect(target.querySelector('pre')?.textContent).toContain('User: hi');
    unmount(c);
  });

  it('asks for handoff confirmation and sends provider and handoff mode on switch', async () => {
    const viewWithTurn: ThreadView = {
      ...base,
      thread: {
        id: 't',
        workspace: 'w',
        provider: 'p.a/a',
        model: 'm1',
        status: 'idle',
        created: '2026-11-24T00:00:00Z',
        updated: '2026-11-24T00:00:00Z',
      },
      turns: [
        {
          id: 'u1',
          input: 'hello',
          started: '2026-11-24T00:00:01Z',
          ended: '2026-11-24T00:00:02Z',
          outcome: 'done',
          provider: 'p.a/a',
          model: 'm1',
          items: [{ kind: 'message', text: 'hi there', done: true }],
        },
      ],
    };
    const connection = {
      id: 'local',
      ready: true,
      call: async (method: string) => {
        if (method === 'provider.list') {
          return {
            providers: [
              { id: 'p.a/a', name: 'Agent A', plugin: 'p.a' },
              { id: 'p.b/b', name: 'Agent B', plugin: 'p.b' },
            ],
          };
        }
        return {};
      },
      subscribe: () => () => {},
      models: async () => [{ id: 'm1', label: 'Model 1', capabilities: [] }],
    };
    const { ctx, ran } = fakeCtx(viewWithTurn);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address, connection: connection as never } });
    flushSync();
    await Promise.resolve();
    await Promise.resolve();
    flushSync();

    expect(target.textContent).toContain('Agent A · m1');
    const select = target.querySelector<HTMLSelectElement>('select[aria-label="Thread provider"]');
    expect(select).not.toBeNull();
    select!.value = 'p.b/b';
    select!.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();

    expect(target.textContent).toContain('Hand off to Agent B?');
    const summaryRadio = target.querySelector<HTMLInputElement>('input[value="summary"]');
    summaryRadio!.checked = true;
    summaryRadio!.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();
    const confirmBtn = [...target.querySelectorAll<HTMLButtonElement>('button')].find((b) =>
      b.textContent?.trim() === 'Hand off',
    );
    confirmBtn!.click();
    await Promise.resolve();
    flushSync();

    expect(ran).toEqual([['thread.update', { address, provider: 'p.b/b', handoff: 'summary' }]]);
    unmount(c);
  });
});
