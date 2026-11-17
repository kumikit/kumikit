import { describe, expect, it } from 'vitest';

import { CommandRegistry, fuzzyScore, keyMatches, whenMatches } from '../src/commands/registry.svelte';
import { MAX_FAILURES, SlotRegistry } from '../src/slots/registry.svelte';

const key = (k: string, mods: Partial<KeyboardEvent> = {}) =>
  ({ key: k, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...mods }) as KeyboardEvent;

describe('commands', () => {
  it('evaluate when clauses', () => {
    expect(whenMatches(undefined, {})).toBe(true);
    expect(whenMatches('thread.open && !busy', { 'thread.open': true })).toBe(true);
    expect(whenMatches('thread.open', {})).toBe(false);
  });

  it('match keys with Mod per platform', () => {
    expect(keyMatches('Mod+K', key('k', { ctrlKey: true }), false)).toBe(true);
    expect(keyMatches('Mod+K', key('k', { metaKey: true }), true)).toBe(true);
    expect(keyMatches('Mod+K', key('k', { ctrlKey: true, shiftKey: true }), false)).toBe(false);
    expect(keyMatches('Escape', key('Escape'), false)).toBe(true);
  });

  it('rank fuzzy matches', () => {
    expect(fuzzyScore('nt', 'New thread')).toBeGreaterThan(fuzzyScore('nt', 'Open settings'));
    expect(fuzzyScore('xyz', 'New thread')).toBe(-1);
  });

  it('search, run and remove by source', async () => {
    const r = new CommandRegistry();
    let ran = 0;
    r.register({ id: 'thread.new', title: 'New thread' }, () => ++ran);
    r.register({ id: 'p.only', title: 'Plugin thing', when: 'thread.open' }, () => {}, 'p');
    expect(r.search('', {}).map((c) => c.id)).toEqual(['thread.new']);
    expect(r.search('thing', { 'thread.open': true }).map((c) => c.id)).toEqual(['p.only']);
    await r.run('thread.new');
    expect(ran).toBe(1);
    r.removeSource('p');
    expect(r.commands.map((c) => c.id)).toEqual(['thread.new']);
    await expect(r.run('p.only')).rejects.toThrow('no command');
  });
});

describe('slots', () => {
  it('find renderers, with the plugin.event fallback', () => {
    const s = new SlotRegistry();
    const mount = () => {};
    s.register('a', 'transcript.renderer', { kind: 'message', mount });
    s.register('a', 'transcript.renderer', { kind: 'plugin.event', mount });
    expect(s.renderer('message')?.plugin).toBe('a');
    expect(s.renderer('plugin.event:x.y/plan')?.contribution.kind).toBe('plugin.event');
    expect(s.renderer('tool')).toBeUndefined();
  });

  it(`disable a plugin after ${MAX_FAILURES} failures`, () => {
    const disabled: string[] = [];
    const s = new SlotRegistry((p) => disabled.push(p));
    s.register('bad', 'statusbar.item', { id: 'x', mount: () => {} });
    expect(s.fail('bad', new Error('boom'))).toBe(false);
    expect(s.fail('bad', new Error('boom'))).toBe(false);
    expect(s.fail('bad', new Error('boom'))).toBe(true);
    expect(disabled).toEqual(['bad']);
    expect(s.in('statusbar.item')).toEqual([]);
  });
});
