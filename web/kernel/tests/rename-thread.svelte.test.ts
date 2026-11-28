// @vitest-environment jsdom
/**
 * RenameThread keyboard behavior: Enter submits the form (ignoring IME composition),
 * Escape closes, and pressing Enter while Cancel is focused does not save.
 */
import type { ThreadAddress } from '@kumikit/ui-sdk';
import { flushSync, mount, unmount } from 'svelte';
import { describe, expect, it, vi } from 'vitest';

import type { Kernel } from '../src/app.svelte';
import RenameThread from '../src/ui/RenameThread.svelte';

const address: ThreadAddress = { connection: 'local', workspace: 'w1', thread: 't1' };

function openDialog(title = 'Old title') {
  const saveThreadTitle = vi.fn(async () => true);
  const kernel = {
    dialog: { kind: 'rename', address, title },
    saveThreadTitle,
  } as unknown as Kernel;
  const target = document.createElement('div');
  document.body.append(target);
  const component = mount(RenameThread, { target, props: { kernel, address, title } });
  flushSync();
  return {
    kernel,
    saveThreadTitle,
    target,
    cleanup() {
      unmount(component);
      target.remove();
    },
  };
}

describe('RenameThread', () => {
  it('does not save when Enter is pressed on the Cancel button', () => {
    const { kernel, saveThreadTitle, target, cleanup } = openDialog();
    const input = target.querySelector<HTMLInputElement>('input')!;
    input.value = 'New title';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    const cancel = Array.from(target.querySelectorAll('button')).find((b) => b.textContent?.trim() === 'Cancel')!;
    expect(cancel.type).toBe('button');
    cancel.focus();
    cancel.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', bubbles: true, cancelable: true }));
    cancel.click();
    flushSync();

    expect(saveThreadTitle).not.toHaveBeenCalled();
    expect(kernel.dialog).toBeUndefined();
    cleanup();
  });

  it('saves when the form is submitted with a changed title and ignores IME Enter', async () => {
    const { kernel, saveThreadTitle, target, cleanup } = openDialog();
    const input = target.querySelector<HTMLInputElement>('input')!;
    input.value = '  Renamed thread  ';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    const composing = new KeyboardEvent('keydown', {
      key: 'Enter',
      isComposing: true,
      bubbles: true,
      cancelable: true,
    });
    input.dispatchEvent(composing);
    expect(composing.defaultPrevented).toBe(true);
    expect(saveThreadTitle).not.toHaveBeenCalled();

    const form = target.querySelector<HTMLFormElement>('form')!;
    form.dispatchEvent(new Event('submit', { bubbles: true, cancelable: true }));
    await Promise.resolve();
    flushSync();

    expect(saveThreadTitle).toHaveBeenCalledWith(address, 'Renamed thread');
    expect(kernel.dialog).toBeUndefined();
    cleanup();
  });

  it('closes without saving on Escape', () => {
    const { kernel, saveThreadTitle, target, cleanup } = openDialog();
    const input = target.querySelector<HTMLInputElement>('input')!;
    input.value = 'Discarded';
    input.dispatchEvent(new Event('input', { bubbles: true }));
    flushSync();

    input.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    flushSync();

    expect(saveThreadTitle).not.toHaveBeenCalled();
    expect(kernel.dialog).toBeUndefined();
    cleanup();
  });
});
