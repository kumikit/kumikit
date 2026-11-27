// @vitest-environment jsdom
import type { ModelInfo, ProviderInfo } from '@kumikit/protocol';
import type { ModelPick } from '@kumikit/ui-sdk';
import { flushSync, mount, unmount } from 'svelte';
import { describe, expect, it } from 'vitest';

import {
  keepValidOptions,
  optionChoices,
  orderOptions,
  providerInitial,
  pushRecent,
  searchModels,
} from '../src/ui/model-picker';
import ModelPicker from '../src/ui/ModelPicker.svelte';

const effort = {
  id: 'effort',
  label: 'Effort',
  kind: 'effort',
  values: [
    { value: 'default', label: 'Default' },
    { value: 'low', label: 'Low' },
    { value: 'high', label: 'High', description: 'Think harder' },
  ],
};
const speed = { id: 'speed', label: 'Speed', kind: 'speed', values: [{ value: 'fast', label: 'Fast' }] };
const other = { id: 'tone', label: 'Tone', values: [{ value: 'terse', label: 'Terse' }] };

describe('model picker helpers', () => {
  it('keeps recent picks unique, newest first and bounded', () => {
    let list = [{ provider: 'a', model: '1' }];
    list = pushRecent(list, { provider: 'b', model: '2' });
    list = pushRecent(list, { provider: 'a', model: '1' });
    expect(list).toEqual([
      { provider: 'a', model: '1' },
      { provider: 'b', model: '2' },
    ]);
    for (let i = 0; i < 20; i++) list = pushRecent(list, { provider: 'c', model: String(i) });
    expect(list).toHaveLength(8);
  });

  it('orders effort, then speed, then the rest, and offers Default for unset', () => {
    expect(orderOptions([other, speed, effort]).map((o) => o.id)).toEqual(['effort', 'speed', 'tone']);
    // A provider value named `default` stands for unset; otherwise Default is added.
    expect(optionChoices(effort).map((c) => [c.value, c.label])).toEqual([
      [undefined, 'Default'],
      ['low', 'Low'],
      ['high', 'High'],
    ]);
    expect(optionChoices(speed).map((c) => [c.value, c.label])).toEqual([
      [undefined, 'Default'],
      ['fast', 'Fast'],
    ]);
  });

  it('keeps only settings the new model offers', () => {
    const model: ModelInfo = { id: 'm', label: 'M', capabilities: [], options: [effort] };
    expect(keepValidOptions(model, { effort: 'high', speed: 'fast' })).toEqual({ effort: 'high' });
    expect(keepValidOptions(model, { effort: 'max' })).toEqual({});
    expect(keepValidOptions(undefined, { effort: 'high' })).toEqual({});
  });

  it('searches label, id and agent name by every word', () => {
    const a: ProviderInfo = { id: 'a', name: 'Claude Code', plugin: 'x' };
    const b: ProviderInfo = { id: 'b', name: 'Antigravity', plugin: 'y' };
    const entries = [
      { provider: a, model: { id: 'opus', label: 'Opus 5.5', capabilities: [] } },
      { provider: b, model: { id: 'claude-opus', label: 'Claude Opus 5.5', capabilities: [] } },
      { provider: b, model: { id: 'gemini', label: 'Gemini 4', capabilities: [] } },
    ];
    expect(searchModels(entries, 'opus').map((e) => e.provider.id)).toEqual(['a', 'b']);
    expect(searchModels(entries, 'opus antigravity').map((e) => e.model.id)).toEqual(['claude-opus']);
    expect(providerInitial(' codex')).toBe('C');
  });
});

describe('the model picker', () => {
  const providers: ProviderInfo[] = [
    { id: 'p.a/a', name: 'Agent A', plugin: 'p.a', icon: 'data:image/svg+xml;base64,PHN2Zy8+' },
    { id: 'p.b/b', name: 'Bee', plugin: 'p.b' },
  ];
  const models: Record<string, ModelInfo[]> = {
    'p.a/a': [
      { id: 'a1', label: 'Alpha One', capabilities: [], options: [effort, speed] },
      { id: 'a2', label: 'Alpha Two', capabilities: [] },
    ],
    'p.b/b': [{ id: 'b1', label: 'Bee One', capabilities: [] }],
  };

  async function open(initial: { provider?: string; model?: string; options: Record<string, string> }, recents = [] as { provider: string; model: string }[]) {
    const anchor = document.createElement('button');
    document.body.append(anchor);
    const target = document.createElement('div');
    document.body.append(target);
    const picks: (ModelPick | undefined)[] = [];
    const c = mount(ModelPicker, {
      target,
      props: {
        providers,
        loadModels: async (p: string) => models[p] ?? [],
        initial,
        recents,
        anchor,
        onDone: (p: ModelPick | undefined) => picks.push(p),
      },
    });
    flushSync();
    await Promise.resolve();
    await Promise.resolve();
    flushSync();
    const q = <T extends Element>(s: string) => target.querySelector<T>(s);
    const buttons = (scope: string) => [...target.querySelectorAll<HTMLButtonElement>(`${scope} button`)];
    const done = () => {
      unmount(c);
      target.remove();
      anchor.remove();
    };
    return { target, picks, q, buttons, done };
  }

  it('shows agents with icons or initials, the current model, and its settings', async () => {
    const { q, buttons, picks, done } = await open({ provider: 'p.a/a', model: 'a1', options: { effort: 'low' } });
    expect(q('[role="dialog"]')?.getAttribute('aria-label')).toBe('Choose a model');
    const agents = buttons('nav[aria-label="Agents"]');
    expect(agents.map((b) => b.textContent?.replace(/\s+/g, ' ').trim())).toEqual(['Agent A (current)', 'B Bee']);
    expect(agents[0]!.querySelector('img')?.getAttribute('src')).toBe(providers[0]!.icon);
    const listed = buttons('section[aria-label="Models"]');
    expect(listed.map((b) => b.textContent?.trim())).toEqual(['Alpha One', 'Alpha Two']);
    expect(listed[0]!.getAttribute('aria-pressed')).toBe('true');

    // Settings rows: Effort then Speed; Low is checked; picking High keeps the picker open.
    const rows = [...(q('[role="group"]')?.querySelectorAll('[role="radiogroup"]') ?? [])];
    expect(rows.map((r) => r.getAttribute('aria-label'))).toEqual(['Effort', 'Speed']);
    const high = [...rows[0]!.querySelectorAll<HTMLButtonElement>('[role="radio"]')].find((b) => b.textContent?.trim() === 'High')!;
    expect(rows[0]!.querySelector('[aria-checked="true"]')?.textContent?.trim()).toBe('Low');
    high.click();
    flushSync();
    expect(high.getAttribute('aria-checked')).toBe('true');
    expect(q('.hint')?.textContent).toBe('Think harder');
    expect(picks).toEqual([]);

    // Escape commits what changed.
    q<HTMLElement>('[role="dialog"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(picks).toEqual([{ provider: 'p.a/a', model: 'a1', options: { effort: 'high' } }]);
    done();
  });

  it('switches agent and picks a model, dropping settings it does not offer', async () => {
    const { buttons, picks, done } = await open({ provider: 'p.a/a', model: 'a1', options: { effort: 'high' } });
    buttons('nav[aria-label="Agents"]')[1]!.click();
    await Promise.resolve();
    await Promise.resolve();
    flushSync();
    const listed = buttons('section[aria-label="Models"]');
    expect(listed.map((b) => b.textContent?.trim())).toEqual(['Bee One']);
    listed[0]!.click();
    expect(picks).toEqual([{ provider: 'p.b/b', model: 'b1', options: {} }]);
    done();
  });

  it('lists recent models and searches every agent; Enter picks the highlighted one', async () => {
    const { q, buttons, picks, done } = await open({ options: {} }, [{ provider: 'p.b/b', model: 'b1' }]);
    // Without a current agent the picker opens on Recent.
    expect(buttons('nav[aria-label="Agents"]')[0]!.getAttribute('aria-current')).toBe('true');
    expect(buttons('section[aria-label="Models"]').map((b) => b.textContent?.replace(/\s+/g, ' ').trim())).toEqual(['B Bee One Bee']);

    const search = q<HTMLInputElement>('input[type="search"]')!;
    search.value = 'two';
    search.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();
    await Promise.resolve();
    await Promise.resolve();
    flushSync();
    expect(buttons('section[aria-label="Models"]').map((b) => b.title)).toEqual(['a2']);
    search.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    expect(picks).toEqual([{ provider: 'p.a/a', model: 'a2', options: {} }]);
    done();
  });

  it('closes without a pick when nothing changed', async () => {
    const { q, picks, done } = await open({ provider: 'p.a/a', model: 'a1', options: {} });
    q<HTMLElement>('[role="dialog"]')!.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    expect(picks).toEqual([undefined]);
    done();
  });
});
