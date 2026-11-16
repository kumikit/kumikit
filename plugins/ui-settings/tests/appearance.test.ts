// @vitest-environment jsdom
import type { PluginContext, ThemeChoiceInfo } from '@kumikit/ui-sdk';
import { flushSync, mount, unmount } from 'svelte';
import { expect, it } from 'vitest';

import Appearance from '../src/Appearance.svelte';

it('switches between following the system, a mode and a theme variant', () => {
  let choice: ThemeChoiceInfo = { variant: null, appearance: 'system', reduceMotion: false };
  const listeners = new Set<() => void>();
  const ctx = {
    theme: {
      get choice() {
        return choice;
      },
      variants: [{ key: 'kumikit.theme-default:dark', name: 'Kumikit Dark', appearance: 'dark' }],
      select(c: Partial<ThemeChoiceInfo>) {
        choice = { ...choice, ...c };
        listeners.forEach((l) => l());
      },
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
  const radios = [...target.querySelectorAll<HTMLInputElement>('input[type=radio]')];
  expect(radios.map((r) => r.value)).toEqual(['mode:system', 'mode:light', 'mode:dark', 'kumikit.theme-default:dark']);
  expect(radios[0]!.checked).toBe(true);
  radios[3]!.click();
  flushSync();
  expect(choice.variant).toBe('kumikit.theme-default:dark');
  radios[1]!.click();
  flushSync();
  expect(choice).toEqual({ variant: null, appearance: 'light', reduceMotion: false });
  target.querySelector<HTMLInputElement>('input[type=checkbox]')!.click();
  expect(choice.reduceMotion).toBe(true);
  unmount(c);
  target.remove();
});
