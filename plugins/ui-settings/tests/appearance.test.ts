// @vitest-environment jsdom
import type { PluginContext, ThemeChoiceInfo, ThemeVariantInfo } from '@kumikit/ui-sdk';
import { flushSync, mount, unmount } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import Appearance from '../src/Appearance.svelte';

describe('Appearance', () => {
  it('switches modes, filters bundled themes, shows preview thumbnails, and imports/removes custom themes', async () => {
    let choice: ThemeChoiceInfo = { variant: null, appearance: 'system', reduceMotion: false };
    let variants: ThemeVariantInfo[] = [
      {
        key: 'kumikit-light',
        name: 'Kumikit Light',
        appearance: 'light',
        source: 'bundled',
        preview: { 'surface.panel': '#ffffff', accent: '#3556c4' },
      },
      {
        key: 'tokyo-night',
        name: 'Tokyo Night',
        appearance: 'dark',
        source: 'bundled',
        preview: { 'surface.panel': '#1a1b26', accent: '#7aa2f7' },
      },
    ];
    const listeners = new Set<() => void>();
    const notify = () => listeners.forEach((l) => l());
    const importFromFile = vi.fn(async () => {
      const added: ThemeVariantInfo = {
        key: 'custom:ocean',
        name: 'Ocean',
        appearance: 'dark',
        source: 'custom',
        preview: { 'surface.panel': '#03045e', accent: '#00b4d8' },
      };
      variants = [...variants, added];
      choice = { ...choice, variant: added.key };
      notify();
      return added;
    });
    const remove = vi.fn((key: string) => {
      variants = variants.filter((v) => v.key !== key);
      if (choice.variant === key) choice = { ...choice, variant: null };
      notify();
    });

    const ctx = {
      theme: {
        get choice() {
          return choice;
        },
        get variants() {
          return variants;
        },
        select(c: Partial<ThemeChoiceInfo>) {
          choice = { ...choice, ...c };
          notify();
        },
        importFromFile,
        remove,
        onChange(fn: () => void) {
          listeners.add(fn);
          return () => listeners.delete(fn);
        },
      },
    } as unknown as PluginContext;

    const target = document.createElement('div');
    document.body.append(target);
    const c = mount(Appearance, { target, props: { ctx } });
    flushSync();

    // Renders SVG preview thumbnails with the theme's palette colors.
    const svgs = target.querySelectorAll('svg.thumb');
    expect(svgs).toHaveLength(2);
    expect(svgs[1]!.innerHTML).toContain('#1a1b26');
    expect(svgs[1]!.innerHTML).toContain('#7aa2f7');

    // Mode + theme selection.
    const radio = (v: string) => target.querySelector<HTMLInputElement>(`input[type=radio][value="${v}"]`)!;
    expect(radio('mode:system').checked).toBe(true);
    radio('tokyo-night').click();
    flushSync();
    expect(choice.variant).toBe('tokyo-night');

    // Filter bundled themes by appearance.
    const lightFilter = [...target.querySelectorAll<HTMLButtonElement>('.filter-btn')].find((b) => b.textContent?.trim() === 'Light')!;
    lightFilter.click();
    flushSync();
    expect(target.querySelectorAll('svg.thumb')).toHaveLength(1);
    expect(target.textContent).toContain('Kumikit Light');
    expect(target.textContent).not.toContain('Tokyo Night');

    // Import a custom theme from disk, then remove it.
    target.querySelector<HTMLButtonElement>('.add-btn')!.click();
    await Promise.resolve();
    flushSync();
    expect(importFromFile).toHaveBeenCalledTimes(1);
    expect(target.textContent).toContain('Your themes (1)');
    expect(target.textContent).toContain('Ocean');
    expect(choice.variant).toBe('custom:ocean');

    target.querySelector<HTMLButtonElement>('button[aria-label="Remove Ocean"]')!.click();
    flushSync();
    expect(remove).toHaveBeenCalledWith('custom:ocean');
    expect(target.textContent).not.toContain('Your themes');

    // Mode and motion toggles still work.
    radio('mode:light').click();
    flushSync();
    expect(choice).toEqual({ variant: null, appearance: 'light', reduceMotion: false });
    target.querySelector<HTMLInputElement>('input[type=checkbox]')!.click();
    expect(choice.reduceMotion).toBe(true);

    unmount(c);
    target.remove();
  });
});
