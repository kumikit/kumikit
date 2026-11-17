// @vitest-environment jsdom
import { describe, expect, it } from 'vitest';

import { Kernel } from '../src/app.svelte';
import { renderMarkdown } from '../src/declarative/markdown';
import { FakeCore, settle } from './fake-core';

describe('links', () => {
  it('are copied, never followed', async () => {
    const kernel = new Kernel({ platform: new FakeCore().platform() });
    const copied: string[] = [];
    const stop = kernel.guardLinks(document, { writeText: async (t) => void copied.push(t) });
    const el = document.createElement('div');
    document.body.append(el);
    renderMarkdown(el, '[docs](https://example.com/docs)');
    const a = el.querySelector('a')!;
    const click = new MouseEvent('click', { bubbles: true, cancelable: true });
    a.dispatchEvent(click);
    await settle();
    expect(click.defaultPrevented).toBe(true);
    expect(copied).toEqual(['https://example.com/docs']);
    expect(kernel.toasts.at(-1)?.message).toBe('Link copied: https://example.com/docs');

    const plain = document.createElement('a');
    plain.href = 'https://elsewhere.example';
    el.append(plain);
    const other = new MouseEvent('click', { bubbles: true, cancelable: true });
    plain.dispatchEvent(other);
    expect(other.defaultPrevented).toBe(true);
    expect(copied).toHaveLength(1);
    stop();
    el.remove();
  });
});
