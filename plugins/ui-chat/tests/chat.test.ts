// @vitest-environment jsdom
import type { PluginContext, RendererProps, ThreadView } from '@kumikit/ui-sdk';
import { flushSync, mount, unmount } from 'svelte';
import { describe, expect, it } from 'vitest';

import Approval from '../src/Approval.svelte';
import Chat from '../src/Chat.svelte';
import Handoff from '../src/Handoff.svelte';
import SideThread from '../src/SideThread.svelte';
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
    const requests: unknown[] = [];
    ctx.ui.pickModel = async (_anchor, request) => {
      requests.push(request);
      return { provider: 'p.b/b', model: 'm9', options: { effort: 'high' } };
    };
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address, connection: connection as never } });
    flushSync();
    await Promise.resolve();
    await Promise.resolve();
    flushSync();

    expect(target.textContent).toContain('Agent A · m1');
    const button = target.querySelector<HTMLButtonElement>('button[aria-label^="Model:"]')!;
    expect(button.getAttribute('aria-label')).toBe('Model: Agent A · Model 1');
    button.click();
    await Promise.resolve();
    await Promise.resolve();
    flushSync();
    expect(requests).toEqual([{ connection: 'local', provider: 'p.a/a', model: 'm1', options: {} }]);

    expect(target.textContent).toContain('Hand off to Agent B?');
    expect(ran).toEqual([]);
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

    expect(ran).toEqual([
      ['thread.update', { address, provider: 'p.b/b', model: 'm9', options: { effort: 'high' }, handoff: 'summary' }],
    ]);
    unmount(c);
  });

  it('changes the model and its settings on the same agent without a handoff', async () => {
    const view: ThreadView = {
      ...base,
      thread: {
        id: 't',
        workspace: 'w',
        provider: 'p.a/a',
        model: 'm1',
        options: { effort: 'low' },
        status: 'idle',
        created: '2026-11-24T00:00:00Z',
        updated: '2026-11-24T00:00:00Z',
      },
    };
    const effort = {
      id: 'effort',
      label: 'Effort',
      kind: 'effort',
      values: [
        { value: 'low', label: 'Low' },
        { value: 'high', label: 'High' },
      ],
    };
    const connection = {
      id: 'local',
      ready: true,
      call: async () => ({ providers: [{ id: 'p.a/a', name: 'Agent A', plugin: 'p.a', icon: 'data:image/svg+xml;base64,PHN2Zy8+' }] }),
      subscribe: () => () => {},
      models: async () => [
        { id: 'm1', label: 'Model 1', capabilities: [], options: [effort] },
        { id: 'm2', label: 'Model 2', capabilities: [], options: [effort] },
      ],
    };
    const { ctx, ran } = fakeCtx(view);
    ctx.ui.pickModel = async () => ({ provider: 'p.a/a', model: 'm2', options: { effort: 'high' } });
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address, connection: connection as never } });
    flushSync();
    await Promise.resolve();
    await Promise.resolve();
    flushSync();

    const button = target.querySelector<HTMLButtonElement>('button[aria-label^="Model:"]')!;
    // The agent's icon and the chosen effort show on the button.
    expect(button.querySelector('img')?.getAttribute('src')).toBe('data:image/svg+xml;base64,PHN2Zy8+');
    expect(button.textContent).toContain('Model 1 · Low');
    button.click();
    await Promise.resolve();
    await Promise.resolve();
    flushSync();
    expect(ran).toEqual([['thread.update', { address, model: 'm2', options: { effort: 'high' } }]]);
    expect(target.textContent).not.toContain('Hand off to');
    unmount(c);
  });
});

describe('deferred messages UI', () => {
  it('shows trigger chips and Run now on deferred queue entries and parses /later', () => {
    const viewWithDefer: ThreadView = {
      ...base,
      queue: [
        {
          id: 'q1',
          turn: 'u2',
          input: { text: 'check deploy' },
          trigger: { kind: 'after_command', argv: ['cargo', 'test'], timeoutS: 600 },
        },
      ],
    };
    const { ctx, ran } = fakeCtx(viewWithDefer);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();

    expect(target.textContent).toContain('After `cargo test`');
    expect(target.textContent).toContain('Run now');

    const ta = target.querySelector<HTMLTextAreaElement>('textarea')!;
    ta.value = '/later after cargo build -- review output';
    ta.dispatchEvent(new Event('input', { bubbles: true }));
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    flushSync();

    expect(ran).toEqual([
      [
        'thread.defer',
        {
          address,
          text: 'review output',
          trigger: { kind: 'after_command', argv: ['cargo', 'build'], timeoutS: 600 },
        },
      ],
    ]);
    unmount(c);
  });
});

describe('forks, edit-and-resend, and side threads', () => {
  it('edits a sent prompt by forking beforeTurn, saving preload draft, and opening the fork', async () => {
    const viewWithTurn: ThreadView = {
      ...base,
      turns: [
        {
          id: 'u1',
          input: 'original prompt',
          started: '2026-11-15T10:00:00Z',
          ended: '2026-11-15T10:00:02Z',
          outcome: 'done',
          items: [{ kind: 'message', text: 'reply one', done: true }],
        },
      ],
    };
    const stored = new Map<string, string>();
    const ran: [string, unknown][] = [];
    const { ctx } = fakeCtx(viewWithTurn);
    ctx.storage = {
      get: (k) => stored.get(k),
      set: (k, v) => {
        stored.set(k, v);
      },
      delete: (k) => {
        stored.delete(k);
      },
    };
    ctx.commands = {
      register: () => () => {},
      run: async (cmd, args) => {
        ran.push([cmd, args]);
        if (cmd === 'thread.fork') return { id: 'th-fork' };
        return undefined;
      },
    };
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();

    const editBtn = target.querySelector<HTMLButtonElement>('button[aria-label="Edit prompt in fork"]')!;
    editBtn.click();
    await Promise.resolve();
    await Promise.resolve();

    expect(ran[0]).toEqual(['thread.fork', { address, beforeTurn: 'u1', open: false }]);
    expect(ran[1]).toEqual(['thread.open', { address: { ...address, thread: 'th-fork' } }]);
    const preload = JSON.parse(stored.get('preload:th-fork')!);
    expect(preload.text).toBe('original prompt');
    expect(preload.note).toContain('not rewound');
    unmount(c);
  });

  it('offers Insert into main in side-thread mode and /btw opens side inspector', async () => {
    const parent: ThreadAddress = { connection: 'local', workspace: 'w', thread: 'main-th' };
    const sideView: ThreadView = {
      ...base,
      address: { connection: 'local', workspace: 'w', thread: 'side-th' },
      turns: [
        {
          id: 's1',
          input: 'side q',
          started: '2026-11-15T10:00:00Z',
          ended: '2026-11-15T10:00:01Z',
          outcome: 'done',
          items: [{ kind: 'message', text: 'useful side answer', done: true }],
        },
      ],
    };
    const { ctx, ran } = fakeCtx(sideView);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address: sideView.address, sideFor: parent } });
    flushSync();

    const insertBtn = Array.from(target.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Insert into main'),
    )!;
    expect(insertBtn).toBeTruthy();
    insertBtn.click();
    flushSync();

    expect(ran).toEqual([['composer.insert', { address: parent, text: 'useful side answer' }]]);
    unmount(c);
  });
});

describe('composer and transcript polish (M7 step 7)', () => {
  it('shows slash command menu (agent + client), inserts without running, and supports Approve all + grouped tools + changed files', async () => {
    const toolItems = Array.from({ length: 5 }, (_, i) => ({
      kind: 'tool' as const,
      call: `c${i}`,
      name: `tool_${i}`,
      input: {},
      output: '',
      ok: i !== 2,
    }));
    const richView: ThreadView = {
      ...base,
      status: 'awaiting_approval',
      commands: [{ name: 'review', description: 'Review changes' }],
      pendingApprovals: [
        { id: 'ap1', thread: 'th', turn: 'u1', kind: 'tool', summary: 'Run tests', status: 'pending' },
        { id: 'ap2', thread: 'th', turn: 'u1', kind: 'tool', summary: 'Write file', status: 'pending' },
      ],
      turns: [
        {
          id: 'u1',
          input: 'check everything',
          started: '2026-11-15T10:00:00Z',
          ended: '2026-11-15T10:00:05Z',
          outcome: 'done',
          items: [
            ...toolItems,
            { kind: 'file.changed', path: 'src/app.ts', diff: '--- a/src/app.ts\n+++ b/src/app.ts\n-old\n+new\n+more' },
            { kind: 'message', text: 'All checked.', done: true },
          ],
        },
      ],
    };
    const { ctx, ran } = fakeCtx(richView);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();

    // A finished turn collapses its tool calls under one header with the failure count.
    const activity = target.querySelector<HTMLDetailsElement>('details.activity')!;
    expect(activity.querySelector('summary')!.textContent).toContain('Used 5 tools · 1 failed');
    expect(activity.open).toBe(false);
    expect(activity.querySelectorAll('.item')).toHaveLength(5);
    // The reply stays visible below it.
    expect(target.querySelector('.summary')!.textContent).toContain('All checked.');
    // Per-turn changed files footer with +/- counts.
    const footer = target.querySelector<HTMLButtonElement>('button.changes-footer')!;
    expect(footer.textContent).toContain('1 changed file');
    expect(footer.textContent).toContain('+2');
    expect(footer.textContent).toContain('−1');
    // Elapsed time shown on turn.
    expect(target.textContent).toContain('5s');

    // Approve all calls approval.respond once per pending approval.
    const approveAllBtn = Array.from(target.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Approve all'),
    )!;
    approveAllBtn.click();
    await Promise.resolve();
    await Promise.resolve();
    expect(ran).toEqual([
      ['approval.respond', { address, approval: 'ap1', decision: 'allow' }],
      ['approval.respond', { address, approval: 'ap2', decision: 'allow' }],
    ]);

    // Typing `/rev` opens the slash menu; Enter inserts `/review ` without sending.
    const ta = target.querySelector<HTMLTextAreaElement>('textarea')!;
    ta.value = '/rev';
    ta.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    expect(target.querySelector('[role="listbox"][aria-label="Slash commands"]')?.textContent).toContain('/review');
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    flushSync();
    expect(ta.value).toBe('/review ');
    expect(ran.length).toBe(2);

    unmount(c);
  });
});

describe('turn summary card and composer stop', () => {
  const finished = (items: ThreadView['turns'][number]['items']): ThreadView => ({
    ...base,
    turns: [{ id: 'u1', input: 'edit', started: '2026-11-15T10:00:00Z', ended: '2026-11-15T10:00:09Z', outcome: 'done', items }],
  });

  it('opens a dialog with only this turn\'s diffs, in unified and split views', () => {
    const { ctx } = fakeCtx(
      finished([
        { kind: 'tool', call: 'c1', name: 'Edit', input: { path: 'a.ts' }, ok: true },
        { kind: 'file.changed', path: 'src/a.ts', diff: '@@ -1,2 +1,2 @@\n keep\n-old\n+new' },
        { kind: 'file.changed', path: 'b.md', diff: '+hello' },
        { kind: 'message', text: 'Edited two files.', done: true },
      ]),
    );
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();

    // Changes are not repeated inline in a finished turn.
    expect(target.querySelector('details.activity')!.querySelectorAll('.item')).toHaveLength(1);
    expect(target.querySelector('dialog')).toBeNull();
    const footer = target.querySelector<HTMLButtonElement>('button.changes-footer')!;
    expect(footer.textContent).toContain('2 changed files');
    footer.click();
    flushSync();

    const dialog = target.querySelector('dialog')!;
    expect(dialog.hasAttribute('open')).toBe(true);
    expect(dialog.querySelector('h2')!.textContent).toBe('Changed files');
    const files = Array.from(dialog.querySelectorAll<HTMLButtonElement>('ul[aria-label="Files"] button'));
    expect(files.map((b) => b.dataset.path)).toEqual(['src/a.ts', 'b.md']);
    const diff = () => dialog.querySelector('section.diff')!;
    expect(diff().getAttribute('aria-label')).toBe('Changes to src/a.ts');
    expect(Array.from(diff().querySelectorAll('tr.del td')).map((td) => td.textContent)).toEqual(['2', '', '-old']);
    expect(Array.from(diff().querySelectorAll('tr.add td')).map((td) => td.textContent)).toEqual(['', '2', '+new']);

    const seg = (name: string) => Array.from(dialog.querySelectorAll<HTMLButtonElement>('button.seg')).find((b) => b.textContent === name)!;
    seg('Split').click();
    flushSync();
    expect(seg('Split').getAttribute('aria-pressed')).toBe('true');
    const pair = Array.from(diff().querySelectorAll('table.split tr')).find((tr) => tr.querySelector('td.code.del'))!;
    expect(Array.from(pair.querySelectorAll('td.code')).map((td) => td.textContent)).toEqual(['old', 'new']);
    seg('Unified').click();
    flushSync();

    files[1]!.click();
    flushSync();
    expect(diff().getAttribute('aria-label')).toBe('Changes to b.md');

    dialog.querySelector<HTMLButtonElement>('button[aria-label="Close"]')!.click();
    flushSync();
    expect(target.querySelector('dialog')).toBeNull();
    unmount(c);
    target.remove();
  });

  it('keeps approvals visible and shows a lone reply without an activity header', () => {
    const { ctx } = fakeCtx(finished([{ kind: 'message', text: 'Just an answer.', done: true }]));
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();
    expect(target.querySelector('details.activity')).toBeNull();
    expect(target.querySelector('.summary')!.textContent).toContain('Just an answer.');
    expect(target.querySelector('button.changes-footer')).toBeNull();
    unmount(c);
  });

  it('swaps Send for Stop while a turn runs and back when it ends', () => {
    const { ctx, ran, push } = fakeCtx(base);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();
    const btn = (label: string) => target.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
    expect(btn('Send')).toBeTruthy();
    expect(btn('Stop')).toBeNull();

    push({ ...base, status: 'running', turns: [{ id: 'u1', input: 'go', started: 't', items: [] }] });
    flushSync();
    expect(btn('Send')).toBeNull();
    btn('Stop')!.click();
    expect(ran).toEqual([['thread.cancel', undefined]]);

    push({ ...base, status: 'idle', turns: [{ id: 'u1', input: 'go', started: 't', ended: 't', outcome: 'cancelled', items: [] }] });
    flushSync();
    expect(btn('Stop')).toBeNull();
    expect(btn('Send')).toBeTruthy();
    unmount(c);
  });
});

describe('usage limits and post-turn checks (M7 step 8)', () => {
  it('shows the limited banner with Resume now and schedules a failing-only check when opted in', () => {
    const stored = new Map<string, string>([
      [`turncheck:${address.workspace}`, JSON.stringify({ enabled: true, cmd: 'cargo test' })],
    ]);
    const limitedView: ThreadView = {
      ...base,
      status: 'limited',
      limit: { resetAt: '2026-11-15T14:30:00.000Z', message: 'Quota hit' },
      turns: [
        {
          id: 'u1',
          input: 'edit file',
          started: '2026-11-15T10:00:00Z',
          ended: '2026-11-15T10:00:02Z',
          outcome: 'done',
          items: [{ kind: 'file.changed', path: 'src/lib.rs' }],
        },
      ],
    };
    const { ctx, ran } = fakeCtx(limitedView);
    ctx.storage = {
      get: (k) => stored.get(k),
      set: (k, v) => {
        stored.set(k, v);
      },
      delete: (k) => {
        stored.delete(k);
      },
    };
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();

    // Post-turn check auto-scheduled deferred after_command with onlyOnFailure: true.
    expect(ran[0]).toEqual([
      'thread.defer',
      {
        address,
        text: 'The post-turn check failed. Fix the reported issue.',
        trigger: { kind: 'after_command', argv: ['cargo', 'test'], timeoutS: 300, onlyOnFailure: true },
      },
    ]);

    // Limited banner with reset time and Resume now button.
    expect(target.textContent).toContain('Usage limit reached');
    expect(target.textContent).toContain('14:30 UTC');
    const resumeBtn = Array.from(target.querySelectorAll('button')).find((b) =>
      b.textContent?.includes('Resume now'),
    )!;
    resumeBtn.click();
    flushSync();
    expect(ran[1]).toEqual(['thread.queue.resume', { address }]);
    unmount(c);
  });

  it('uses icon-only buttons for message, composer, and reply actions', () => {
    const viewWithTurn: ThreadView = {
      ...base,
      turns: [
        {
          id: 'u1',
          input: 'check this',
          started: '2026-11-15T10:00:00Z',
          ended: '2026-11-15T10:00:02Z',
          outcome: 'done',
          items: [{ kind: 'message', text: 'looks good', done: true }],
        },
      ],
    };
    const { ctx, ran } = fakeCtx(viewWithTurn);
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();

    for (const label of [
      'Edit prompt in fork',
      'Fork from turn',
      'Copy reply',
      'Fork from here',
      'Fork thread',
      'Send later',
      'Turn check settings',
    ]) {
      const btn = target.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
      expect(btn, `expected button with aria-label="${label}"`).toBeTruthy();
      expect(btn!.querySelector('svg'), `expected SVG icon in "${label}"`).toBeTruthy();
      expect(btn!.textContent?.trim()).toBe('');
    }

    target.querySelector<HTMLButtonElement>('button[aria-label="Fork thread"]')!.click();
    flushSync();
    expect(ran).toEqual([['thread.fork', { address, open: true }]]);
    unmount(c);
  });

  it('lists and switches between multiple side threads in SideThread inspector', async () => {
    const parent: ThreadAddress = { connection: 'local', workspace: 'w', thread: 'main-th' };
    const side1: ThreadView = {
      ...base,
      address: { connection: 'local', workspace: 'w', thread: 'side-1' },
      turns: [
        {
          id: 't1',
          input: 'first question',
          started: '2026-11-15T10:00:00Z',
          ended: '2026-11-15T10:00:01Z',
          outcome: 'done',
          items: [{ kind: 'message', text: 'first answer', done: true }],
        },
      ],
    };
    const side2: ThreadView = {
      ...base,
      address: { connection: 'local', workspace: 'w', thread: 'side-2' },
      turns: [
        {
          id: 't2',
          input: 'second question',
          started: '2026-11-15T10:01:00Z',
          ended: '2026-11-15T10:01:01Z',
          outcome: 'done',
          items: [{ kind: 'message', text: 'second answer', done: true }],
        },
      ],
    };
    const byId: Record<string, ThreadView> = { 'side-1': side1, 'side-2': side2 };
    const { ctx } = fakeCtx(side2);
    ctx.threads = {
      watch: (addr, fn) => {
        fn(byId[addr.thread] ?? side2);
        return () => {};
      },
    };
    const conn = {
      id: 'local',
      ready: true,
      call: async (method: string) => {
        if (method === 'thread.list') {
          return {
            threads: [
              {
                id: 'side-1',
                workspace: 'w',
                provider: 'p',
                model: 'm',
                title: 'First side',
                status: 'idle',
                side: true,
                parent: 'main-th',
                created: '2026-11-15T10:00:00Z',
                updated: '2026-11-15T10:00:05Z',
              },
              {
                id: 'side-2',
                workspace: 'w',
                provider: 'p',
                model: 'm',
                title: 'Second side',
                status: 'running',
                side: true,
                parent: 'main-th',
                created: '2026-11-15T10:01:00Z',
                updated: '2026-11-15T10:01:05Z',
              },
            ],
          };
        }
        return {};
      },
      subscribe: () => () => {},
      models: async () => [],
    } as unknown as import('@kumikit/ui-sdk').Connection;

    const target = document.createElement('div');
    const c = mount(SideThread, { target, props: { ctx, address: parent, connection: conn } });
    await Promise.resolve();
    await Promise.resolve();
    flushSync();

    const select = target.querySelector<HTMLSelectElement>('select[aria-label="Side thread history"]')!;
    expect(select).toBeTruthy();
    expect(select.options).toHaveLength(2);
    expect(select.options[0]!.textContent).toContain('Second side (running)');
    expect(target.textContent).toContain('second answer');

    // Switch to the earlier side thread.
    select.value = 'side-1';
    select.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();
    expect(target.textContent).toContain('first answer');

    // Click New side thread (+) to view the composer and full history list.
    target.querySelector<HTMLButtonElement>('button[aria-label="New side thread"]')!.click();
    flushSync();
    expect(target.textContent).toContain('History (2)');
    expect(target.querySelector('ul[aria-label="Side thread history list"]')?.textContent).toContain('First side');
    unmount(c);
  });
});

describe('the thinking box', () => {
  const at = (items: ThreadView['turns'][number]['items'], ended = true): ThreadView => ({
    ...base,
    turns: [{ id: 'u1', input: 'think', started: '2026-11-15T10:00:00Z', ...(ended ? { ended: '2026-11-15T10:00:09Z', outcome: 'done' } : {}), items }],
  });
  const items = [
    { kind: 'reasoning', text: 'first thought' },
    { kind: 'tool', call: 'c1', name: 'Read', input: { path: 'a.ts' }, ok: true },
    { kind: 'reasoning', text: 'second thought' },
    { kind: 'message', text: 'Done.', done: true },
  ] as ThreadView['turns'][number]['items'];

  it('in a finished turn sits last in the collapsed activity, collapsed, and stays out of the summary', () => {
    const { ctx } = fakeCtx(at(items));
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();

    const box = target.querySelector<HTMLDetailsElement>('details.activity details.thinking')!;
    expect(box).toBeTruthy();
    expect(box.open).toBe(false);
    expect(box.querySelector('summary')!.textContent).toBe('Thinking...');
    expect(box.querySelector('summary svg.chevron')).toBeTruthy();
    expect(box.querySelector('.thinking-body p')!.textContent).toBe('first thought\n\nsecond thought');
    const body = target.querySelector('details.activity .activity-body')!;
    expect(body.lastElementChild).toBe(box);
    expect(target.querySelector('.summary')!.textContent).not.toContain('thought');
    expect(target.querySelector('.summary')!.textContent).toContain('Done.');
    unmount(c);
    target.remove();
  });

  it('moves intermediate narration between tool calls into the Thinking box while running and when finished', () => {
    const narrated = [
      { kind: 'message', text: 'Let me check a.ts.', done: false },
      { kind: 'tool', call: 'c1', name: 'Read', input: { path: 'a.ts' }, ok: true },
      { kind: 'message', text: 'Now checking b.ts.', done: false },
      { kind: 'tool', call: 'c2', name: 'Read', input: { path: 'b.ts' }, ok: true },
      { kind: 'message', text: 'All done.', done: true },
    ] as ThreadView['turns'][number]['items'];

    // While running: only tool calls appear above the Thinking box, not intermediate text.
    const runningTarget = document.createElement('div');
    document.body.append(runningTarget);
    const running = mount(Chat, { target: runningTarget, props: { ctx: fakeCtx(at(narrated.slice(0, 4), false)).ctx, address } });
    flushSync();
    expect(runningTarget.querySelectorAll('.item')).toHaveLength(2);
    const runningBox = runningTarget.querySelector<HTMLDetailsElement>('details.thinking')!;
    expect(runningBox.querySelector('.thinking-body p')!.textContent).toBe('Let me check a.ts.\n\nNow checking b.ts.');
    unmount(running);
    runningTarget.remove();

    // When finished: activity holds only the tool calls + Thinking box; summary holds only the reply.
    const doneTarget = document.createElement('div');
    document.body.append(doneTarget);
    const done = mount(Chat, { target: doneTarget, props: { ctx: fakeCtx(at(narrated, true)).ctx, address } });
    flushSync();
    expect(doneTarget.querySelectorAll('details.activity .activity-body > .item')).toHaveLength(2);
    expect(doneTarget.querySelector('details.activity details.thinking .thinking-body p')!.textContent).toBe(
      'Let me check a.ts.\n\nNow checking b.ts.',
    );
    expect(doneTarget.querySelector('.summary')!.textContent).toContain('All done.');
    expect(doneTarget.querySelector('.summary')!.textContent).not.toContain('Let me check');
    unmount(done);
    doneTarget.remove();
  });

  it('while the turn runs sits beneath the tool calls', () => {
    const { ctx } = fakeCtx(at(items.slice(0, 3), false));
    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();

    const box = target.querySelector<HTMLDetailsElement>('details.thinking')!;
    expect(box.open).toBe(false);
    expect(box.querySelector('.thinking-body p')!.textContent).toBe('first thought\n\nsecond thought');
    const tool = target.querySelector('.item')!;
    expect(tool.compareDocumentPosition(box) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    unmount(c);
    target.remove();
  });

  it('shows a turn that is only thinking as Thinking, with no summary', () => {
    const { ctx } = fakeCtx(at([{ kind: 'reasoning', text: 'just this' }]));
    const target = document.createElement('div');
    const c = mount(Chat, { target, props: { ctx, address } });
    flushSync();
    expect(target.querySelector('details.activity .activity-label')!.textContent).toBe('Thinking');
    expect(target.querySelector('.summary')).toBeNull();
    unmount(c);
  });
});
