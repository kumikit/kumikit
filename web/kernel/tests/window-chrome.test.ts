// @vitest-environment jsdom
/** Borderless desktop window chrome, macOS native overlay inset, and tiling WM button visibility. */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import { Kernel } from '../src/app.svelte';
import type { WindowChrome, WindowControls } from '../src/platform/index';
import { parseSettings } from '../src/settings/settings.svelte';
import { FakeCore, ManualTimers, platformOf, settle } from './fake-core';

function makeWindow(chrome: WindowChrome) {
  let maximized = false;
  let fullscreen = false;
  let onResize: (() => void) | undefined;
  const controls: WindowControls = {
    chrome: async () => chrome,
    minimize: vi.fn(async () => {}),
    toggleMaximize: vi.fn(async () => {
      maximized = !maximized;
      onResize?.();
    }),
    close: vi.fn(async () => {}),
    startDragging: vi.fn(async () => {}),
    startResizeDragging: vi.fn(async () => {}),
    isMaximized: async () => maximized,
    isFullscreen: async () => fullscreen,
    onStateChange: async (cb) => {
      onResize = cb;
      return () => {
        onResize = undefined;
      };
    },
  };
  return {
    controls,
    setMaximized(v: boolean) {
      maximized = v;
      onResize?.();
    },
    setFullscreen(v: boolean) {
      fullscreen = v;
      onResize?.();
    },
  };
}

async function startWith(chrome?: WindowChrome) {
  const local = new FakeCore();
  const base = platformOf({ local });
  const win = chrome ? makeWindow(chrome) : undefined;
  const platform = win ? { ...base, window: win.controls } : base;
  const kernel = new Kernel({ platform, timers: new ManualTimers() });
  await kernel.start();
  return { kernel, win };
}

beforeEach(() => {
  vi.stubGlobal('fetch', async () => new Response('{}'));
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe('window chrome', () => {
  it('hides window buttons by default on a Linux tiling WM and respects Show/Hide overrides', async () => {
    const { kernel } = await startWith({
      borderless: true,
      nativeOverlay: false,
      tiling: true,
      customResize: false,
    });
    expect(kernel.showWindowButtons).toBe(false);
    expect(kernel.showCustomResizeHandles).toBe(false);
    expect(kernel.nativeOverlayInset).toBe(false);
    expect(kernel.canDragTopBar).toBe(true);

    kernel.settings.update({ windowButtons: 'show' });
    expect(kernel.showWindowButtons).toBe(true);

    kernel.settings.update({ windowButtons: 'hide' });
    expect(kernel.showWindowButtons).toBe(false);
  });

  it('shows window buttons and custom resize handles on a Linux stacking DE until maximized', async () => {
    const { kernel, win } = await startWith({
      borderless: true,
      nativeOverlay: false,
      tiling: false,
      customResize: true,
    });
    expect(kernel.showWindowButtons).toBe(true);
    expect(kernel.showCustomResizeHandles).toBe(true);

    win!.setMaximized(true);
    await settle();
    expect(kernel.windowMaximized).toBe(true);
    expect(kernel.showCustomResizeHandles).toBe(false);

    kernel.settings.update({ windowButtons: 'hide' });
    expect(kernel.showWindowButtons).toBe(false);
  });

  it('uses native overlay inset on macOS (without duplicate buttons) and clears inset in fullscreen', async () => {
    const { kernel, win } = await startWith({
      borderless: false,
      nativeOverlay: true,
      tiling: false,
      customResize: false,
    });
    expect(kernel.showWindowButtons).toBe(false);
    expect(kernel.nativeOverlayInset).toBe(true);
    expect(kernel.canDragTopBar).toBe(true);
    expect(kernel.showCustomResizeHandles).toBe(false);

    // Even if windowButtons is set to 'show', macOS native overlay never duplicates buttons.
    kernel.settings.update({ windowButtons: 'show' });
    expect(kernel.showWindowButtons).toBe(false);

    win!.setFullscreen(true);
    await settle();
    expect(kernel.nativeOverlayInset).toBe(false);
  });

  it('shows nothing in browser/relay platforms without window controls', async () => {
    const { kernel } = await startWith(undefined);
    expect(kernel.showWindowButtons).toBe(false);
    expect(kernel.nativeOverlayInset).toBe(false);
    expect(kernel.showCustomResizeHandles).toBe(false);
    expect(kernel.canDragTopBar).toBe(false);
  });

  it('parses windowButtons setting cleanly with auto default', () => {
    expect(parseSettings(null).windowButtons).toBe('auto');
    expect(parseSettings(JSON.stringify({ windowButtons: 'show' })).windowButtons).toBe('show');
    expect(parseSettings(JSON.stringify({ windowButtons: 'hide' })).windowButtons).toBe('hide');
    expect(parseSettings(JSON.stringify({ windowButtons: 'bogus' })).windowButtons).toBe('auto');
  });
});
