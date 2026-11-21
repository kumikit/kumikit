// @vitest-environment jsdom
import type { DeclarativeView, SettingsSchema } from '@kumikit/ui-sdk';
import { flushSync, mount, unmount } from 'svelte';
import { describe, expect, it } from 'vitest';

import Form from '../src/declarative/Form.svelte';
import View from '../src/declarative/View.svelte';
import { checkValue, fields, setPath } from '../src/declarative/schema';

const schema: SettingsSchema = {
  type: 'object',
  required: ['name'],
  properties: {
    name: { type: 'string', title: 'Name', 'x-order': 1 },
    level: { type: 'string', enum: ['low', 'high'], 'x-widget': 'radio', 'x-order': 3 },
    count: { type: 'integer', minimum: 1, maximum: 5, 'x-order': 2 },
    on: { type: 'boolean', default: true },
    token: { type: 'string', 'x-secret': true },
    proxy: { type: 'object', title: 'Proxy', properties: { host: { type: 'string' }, deep: { type: 'object', properties: {} } } },
    list: { type: 'array' as 'string' },
  },
};

describe('settings schemas', () => {
  it('give fields in x-order with groups, and report what they cannot show', () => {
    const r = fields(schema);
    expect(r.fields.map((f) => `${f.key}:${f.kind}`)).toEqual([
      'name:text',
      'count:integer',
      'level:radio',
      'on:boolean',
      'proxy.host:text',
      'token:secret',
    ]);
    expect(r.fields.find((f) => f.key === 'proxy.host')?.group).toBe('Proxy');
    expect(r.errors).toEqual(['list: type array is not supported', 'proxy.deep: only one level of nesting is supported']);
  });

  it('check values', () => {
    const f = fields(schema).fields;
    const by = (k: string) => f.find((x) => x.key === k)!;
    expect(checkValue(by('name'), '')).toBe('Name is required');
    expect(checkValue(by('count'), 6)).toBe('count must be at most 5');
    expect(checkValue(by('count'), 2.5)).toBe('count must be a whole number');
    expect(checkValue(by('level'), 'mid')).toBe('level must be one of the choices');
    expect(checkValue(by('count'), 3)).toBeUndefined();
    expect(setPath({ a: 1 }, 'proxy.host', 'x')).toEqual({ a: 1, proxy: { host: 'x' } });
  });

  it('render a form that passes valid changes on and reports bad ones', () => {
    const target = document.createElement('div');
    const changes: Record<string, unknown>[] = [];
    const c = mount(Form, { target, props: { schema, values: { name: 'a' }, onchange: (v: Record<string, unknown>) => changes.push(v) } });
    const count = target.querySelector<HTMLInputElement>('input[type=number]')!;
    count.value = '9';
    count.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();
    expect(target.querySelector('[role=alert]')?.textContent).toBe('count must be at most 5');
    count.value = '4';
    count.dispatchEvent(new Event('change', { bubbles: true }));
    flushSync();
    expect(changes.at(-1)).toEqual({ name: 'a', count: 4 });
    expect(target.querySelector<HTMLInputElement>('input[type=password]')?.disabled).toBe(true);
    expect(target.querySelector('label[for]')?.textContent).toContain('Name');
    unmount(c);
  });
});

describe('declarative views', () => {
  const cases: [DeclarativeView, string][] = [
    [{ model: 'list', items: [{ id: '1', label: 'One', detail: 'first', command: 'x.open' }] }, 'One first'],
    [{ model: 'tree', items: [{ id: 'a', label: 'A', children: [{ id: 'b', label: 'B' }] }] }, 'AB'],
    [{ model: 'table', columns: [{ key: 'n', title: 'Name' }], rows: [{ n: 'row' }] }, 'Namerow'],
    [{ model: 'markdown', text: '**hi** <b>x</b>' }, 'hi <b>x</b>'],
    [{ model: 'key-value', entries: [{ key: 'Version', value: '1.0' }] }, 'Version1.0'],
    [{ model: 'progress', label: 'Indexing', value: 0.5 }, 'Indexing'],
  ];
  for (const [view, text] of cases) {
    it(`draw ${view.model}`, () => {
      const target = document.createElement('div');
      const c = mount(View, { target, props: { view, run: () => {} } });
      flushSync();
      expect(target.textContent?.replace(/\s+/g, '')).toContain(text.replace(/\s+/g, ''));
      unmount(c);
    });
  }

  it('run a command with the item id', () => {
    const target = document.createElement('div');
    const ran: unknown[] = [];
    mount(View, { target, props: { view: cases[0]![0], run: (c: string, a?: unknown) => ran.push([c, a]) } });
    flushSync();
    target.querySelector('button')!.click();
    expect(ran).toEqual([['x.open', { id: '1' }]]);
  });
});

describe('secret fields of core plugins', () => {
  const core: SettingsSchema = {
    type: 'object',
    properties: { token: { type: 'string', title: 'Token', 'x-secret': 'bot-token' }, name: { type: 'string' } },
  };

  it('write through secret access, never into the values, and show only whether one is set', async () => {
    const target = document.createElement('div');
    const set = new Set<string>();
    const calls: string[] = [];
    const changes: Record<string, unknown>[] = [];
    const secrets = {
      isSet: (n: string) => set.has(n),
      set: async (n: string, v: string) => {
        calls.push(`set ${n} ${v}`);
        set.add(n);
      },
      clear: async (n: string) => {
        calls.push(`clear ${n}`);
        set.delete(n);
      },
    };
    const c = mount(Form, { target, props: { schema: core, values: {}, onchange: (v: Record<string, unknown>) => changes.push(v), secrets } });
    const input = target.querySelector<HTMLInputElement>('input[type=password]')!;
    expect(input.disabled).toBe(false);
    expect(input.placeholder).toBe('Not set');
    input.value = 's3cret-value';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    const save = [...target.querySelectorAll('button')].find((b) => b.textContent === 'Save')!;
    save.click();
    await Promise.resolve();
    await Promise.resolve();
    flushSync();
    expect(calls).toEqual(['set bot-token s3cret-value']);
    expect(changes).toEqual([]);
    expect(input.value).toBe('');
    unmount(c);
  });

  it('stay read-only for UI plugins, which have no core secret to write', () => {
    const target = document.createElement('div');
    const c = mount(Form, { target, props: { schema: core, values: {}, onchange: () => {} } });
    expect(target.querySelector<HTMLInputElement>('input[type=password]')!.disabled).toBe(true);
    unmount(c);
  });
});
