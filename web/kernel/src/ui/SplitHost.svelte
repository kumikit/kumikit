<!-- Recursive split-pane container: renders horizontal (side-by-side) and vertical
     (top-bottom) resizable splits with draggable dividers and keyboard resizing. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import { RATIO_MAX, RATIO_MIN, type LayoutNode } from '../layout/layout';
  import ContextMenu, { type ContextMenuItem } from './ContextMenu.svelte';
  import SplitHost from './SplitHost.svelte';
  import TabBar from './TabBar.svelte';
  import ThreadPane from './ThreadPane.svelte';

  let { kernel, node }: { kernel: Kernel; node: LayoutNode } = $props();

  let container: HTMLElement | undefined = $state();
  let dragging = $state(false);
  let paneMenu = $state<{ x: number; y: number; paneId: string } | undefined>();

  function onPaneContextMenu(e: MouseEvent, paneId: string) {
    const el = e.target as HTMLElement | null;
    if (el?.closest('input, textarea, [contenteditable="true"]')) return;
    e.preventDefault();
    kernel.focusPane(paneId);
    paneMenu = { x: e.clientX, y: e.clientY, paneId };
  }

  const paneMenuItems = $derived.by((): ContextMenuItem[] => {
    const m = paneMenu;
    if (!m) return [];
    const mod = kernel.mac ? '⌘' : 'Ctrl+';
    return [
      {
        id: 'split-h',
        label: 'Split horizontally',
        icon: 'split-h',
        shortcut: `${mod}\\`,
        onSelect: () => kernel.splitPane('horizontal', m.paneId),
      },
      {
        id: 'split-v',
        label: 'Split vertically',
        icon: 'split-v',
        shortcut: `${mod}⇧\\`,
        onSelect: () => kernel.splitPane('vertical', m.paneId),
      },
    ];
  });

  function startDrag(e: PointerEvent, splitId: string, direction: 'horizontal' | 'vertical') {
    if (!container) return;
    e.preventDefault();
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    dragging = true;
    const rect = container.getBoundingClientRect();

    const onMove = (ev: PointerEvent) => {
      const size = direction === 'horizontal' ? rect.width : rect.height;
      if (size <= 0) return;
      const offset = direction === 'horizontal' ? ev.clientX - rect.left : ev.clientY - rect.top;
      kernel.resizeSplit(splitId, offset / size);
    };
    const onEnd = () => {
      dragging = false;
      target.removeEventListener('pointermove', onMove);
      target.removeEventListener('pointerup', onEnd);
      target.removeEventListener('pointercancel', onEnd);
    };
    target.addEventListener('pointermove', onMove);
    target.addEventListener('pointerup', onEnd);
    target.addEventListener('pointercancel', onEnd);
  }

  function onSepKey(e: KeyboardEvent, splitId: string, direction: 'horizontal' | 'vertical', ratio: number) {
    const step = 0.05;
    if (direction === 'horizontal') {
      if (e.key === 'ArrowLeft') kernel.resizeSplit(splitId, ratio - step);
      else if (e.key === 'ArrowRight') kernel.resizeSplit(splitId, ratio + step);
      else return;
    } else {
      if (e.key === 'ArrowUp') kernel.resizeSplit(splitId, ratio - step);
      else if (e.key === 'ArrowDown') kernel.resizeSplit(splitId, ratio + step);
      else return;
    }
    e.preventDefault();
  }

  /** Tracks pointer or keyboard entry into a leaf pane to keep focus synced. */
  function focusPane(el: HTMLElement, id: string) {
    let paneId = id;
    const onEnter = () => kernel.focusPane(paneId);
    el.addEventListener('pointerdown', onEnter);
    el.addEventListener('focusin', onEnter);
    return {
      update(nextId: string) {
        paneId = nextId;
      },
      destroy() {
        el.removeEventListener('pointerdown', onEnter);
        el.removeEventListener('focusin', onEnter);
      },
    };
  }
</script>

{#if node.kind === 'split'}
  <div
    class="split {node.direction}"
    class:dragging
    data-split={node.id}
    bind:this={container}
  >
    <div class="kk-split-first slot">
      <SplitHost {kernel} node={node.first} />
    </div>
    <div
      class="gutter {node.direction}"
      role="slider"
      tabindex="0"
      aria-orientation={node.direction === 'horizontal' ? 'vertical' : 'horizontal'}
      aria-label={node.direction === 'horizontal' ? 'Resize horizontal split' : 'Resize vertical split'}
      aria-valuemin={Math.round(RATIO_MIN * 100)}
      aria-valuemax={Math.round(RATIO_MAX * 100)}
      aria-valuenow={Math.round(node.ratio * 100)}
      title="Drag to resize · Double-click to reset"
      onpointerdown={(e) => startDrag(e, node.id, node.direction)}
      ondblclick={() => kernel.resizeSplit(node.id, 0.5)}
      onkeydown={(e) => onSepKey(e, node.id, node.direction, node.ratio)}
    ></div>
    <div class="kk-split-second slot">
      <SplitHost {kernel} node={node.second} />
    </div>
  </div>
{:else}
  {@const active = kernel.activePaneId === node.id}
  {@const isSplit = kernel.paneCount > 1}
  <div
    class="leaf"
    class:active={isSplit && active}
    class:inactive={isSplit && !active}
    data-pane={node.id}
    use:focusPane={node.id}
  >
    <TabBar {kernel} pane={node} />
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="leaf-body" oncontextmenu={(e) => onPaneContextMenu(e, node.id)}>
      <ThreadPane {kernel} pane={node} />
    </div>
  </div>
{/if}

{#if paneMenu}
  <ContextMenu
    x={paneMenu.x}
    y={paneMenu.y}
    label="Pane actions"
    items={paneMenuItems}
    onClose={() => (paneMenu = undefined)}
  />
{/if}

<style>
  .split {
    display: flex;
    width: 100%;
    height: 100%;
    min-width: 0;
    min-height: 0;
  }
  .split.horizontal {
    flex-direction: row;
  }
  .split.vertical {
    flex-direction: column;
  }
  .split.dragging {
    user-select: none;
  }
  .slot {
    display: flex;
    min-width: 0;
    min-height: 0;
    overflow: hidden;
  }
  .gutter {
    flex: none;
    position: relative;
    background: var(--kk-border-subtle, CanvasText);
    transition: background var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .gutter.horizontal {
    width: 1px;
    cursor: col-resize;
  }
  .gutter.vertical {
    height: 1px;
    cursor: row-resize;
  }
  .gutter::after {
    content: '';
    position: absolute;
    inset: 0;
  }
  .gutter.horizontal::after {
    left: -3px;
    right: -3px;
  }
  .gutter.vertical::after {
    top: -3px;
    bottom: -3px;
  }
  .gutter:hover,
  .gutter:focus-visible {
    background: var(--kk-accent, Highlight);
  }
  .leaf {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    background: var(--kk-surface-panel, Canvas);
  }
  .leaf.active {
    border-top: 2px solid var(--kk-accent, Highlight);
  }
  .leaf.inactive {
    border-top: 2px solid transparent;
  }
  .leaf-body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
