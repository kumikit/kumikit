// @vitest-environment jsdom
/** A slot contribution is mounted once: what it reads while mounting does not make its
 * host remount it (a panel that wrote to its storage was remounted without end). */
import { flushSync, mount, unmount } from 'svelte';
import { describe, expect, it } from 'vitest';

import { SlotRegistry } from '../src/slots/registry.svelte';
import SlotHost from '../src/ui/SlotHost.svelte';

describe('SlotHost', () => {
  it('does not remount when state the contribution read changes', () => {
    const state = $state({ n: 0 });
    let mounts = 0;
    const contribution = {
      mount(el: HTMLElement) {
        mounts++;
        el.textContent = String(state.n);
        return () => {};
      },
    };
    const target = document.createElement('div');
    const host = mount(SlotHost, { target, props: { plugin: 'a.b', contribution, props: {}, slots: new SlotRegistry() } });
    flushSync();
    expect(mounts).toBe(1);
    state.n = 1;
    flushSync();
    state.n = 2;
    flushSync();
    expect(mounts).toBe(1);
    unmount(host);
  });
});
