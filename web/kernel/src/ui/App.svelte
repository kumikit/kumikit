<!-- The window: thin top toolbar, resizable left sidebar, resizable split workspace
     (always mounted underneath overlay sheets), resizable right inspector dock, thin
     bottom status bar, and overlay sheets for Settings, Plugins and Cores. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import { LEFT_MAX, LEFT_MIN, RIGHT_MAX, RIGHT_MIN } from '../layout/layout';
  import type { ResizeDirection } from '../platform/index';
  import ConfirmDialog from './ConfirmDialog.svelte';
  import RenameThread from './RenameThread.svelte';
  import CoreView from './CoreView.svelte';
  import CoresView from './CoresView.svelte';
  import OpenFolder from './OpenFolder.svelte';
  import OverlaySheet from './OverlaySheet.svelte';
  import PairBrowser from './PairBrowser.svelte';
  import Palette from './Palette.svelte';
  import PluginsView from './PluginsView.svelte';
  import RightSidebar from './RightSidebar.svelte';
  import SettingsView from './SettingsView.svelte';
  import Sidebar from './Sidebar.svelte';
  import SlotHost from './SlotHost.svelte';
  import SplitHost from './SplitHost.svelte';
  import StatusBar from './StatusBar.svelte';
  import TabBar from './TabBar.svelte';
  import Toasts from './Toasts.svelte';
  import TopBar from './TopBar.svelte';

  let { kernel }: { kernel: Kernel } = $props();
  // In a browser served by a core, an unpaired browser sees only the pairing screen.
  const pairing = $derived(kernel.platform.browser !== undefined && kernel.connections[0]?.status === 'unauthorized');
  const mainView = $derived(kernel.main);
  const isOverlay = $derived(mainView.kind === 'settings' || mainView.kind === 'plugins' || mainView.kind === 'cores');
  const pluginView = $derived(
    mainView.kind === 'view' ? kernel.slots.in('view').find((v) => v.contribution.id === mainView.id) : undefined,
  );
  let resizing = $state(false);

  // Keep the CSP-safe constructed layout stylesheet synced with sidebar widths & split ratios.
  $effect(() => {
    void kernel.leftSidebarWidth;
    void kernel.rightSidebarWidth;
    void kernel.layoutRoot;
    kernel.applyLayoutStyles();
  });

  function onKey(e: KeyboardEvent) {
    if (e.isComposing || e.keyCode === 229 || kernel.dialog) return;
    const cmd = kernel.commands.forKey(e, kernel.when, kernel.mac, kernel.commandAllowed);
    if (!cmd) return;
    e.preventDefault();
    void kernel.attempt('The command failed', () => Promise.resolve(cmd.run()));
  }

  /** Suppress the default webview context menu (Back/Forward/Reload/Inspect) outside editable text inputs. */
  function onContextMenu(e: MouseEvent) {
    if (e.defaultPrevented) return;
    const target = e.target as HTMLElement | null;
    if (
      target &&
      ((target instanceof HTMLInputElement && !target.readOnly && !target.disabled) ||
        (target instanceof HTMLTextAreaElement && !target.readOnly && !target.disabled) ||
        target.isContentEditable)
    ) {
      return;
    }
    e.preventDefault();
  }

  function dragSidebar(e: PointerEvent, side: 'left' | 'right') {
    e.preventDefault();
    const target = e.currentTarget as HTMLElement;
    target.setPointerCapture(e.pointerId);
    resizing = true;
    const startX = e.clientX;
    const startWidth = side === 'left' ? kernel.leftSidebarWidth : kernel.rightSidebarWidth;

    const onMove = (ev: PointerEvent) => {
      const delta = ev.clientX - startX;
      if (side === 'left') kernel.setLeftSidebarWidth(startWidth + delta);
      else kernel.setRightSidebarWidth(startWidth - delta);
    };
    const onEnd = () => {
      resizing = false;
      target.removeEventListener('pointermove', onMove);
      target.removeEventListener('pointerup', onEnd);
      target.removeEventListener('pointercancel', onEnd);
    };
    target.addEventListener('pointermove', onMove);
    target.addEventListener('pointerup', onEnd);
    target.addEventListener('pointercancel', onEnd);
  }

  function onSidebarSepKey(e: KeyboardEvent, side: 'left' | 'right') {
    const step = 16;
    if (e.key === 'ArrowLeft') {
      if (side === 'left') kernel.setLeftSidebarWidth(kernel.leftSidebarWidth - step);
      else kernel.setRightSidebarWidth(kernel.rightSidebarWidth + step);
    } else if (e.key === 'ArrowRight') {
      if (side === 'left') kernel.setLeftSidebarWidth(kernel.leftSidebarWidth + step);
      else kernel.setRightSidebarWidth(kernel.rightSidebarWidth - step);
    } else {
      return;
    }
    e.preventDefault();
  }
  const RESIZE_EDGES: { dir: ResizeDirection; cls: string }[] = [
    { dir: 'North', cls: 'edge-n' },
    { dir: 'South', cls: 'edge-s' },
    { dir: 'West', cls: 'edge-w' },
    { dir: 'East', cls: 'edge-e' },
    { dir: 'NorthWest', cls: 'edge-nw' },
    { dir: 'NorthEast', cls: 'edge-ne' },
    { dir: 'SouthWest', cls: 'edge-sw' },
    { dir: 'SouthEast', cls: 'edge-se' },
  ];

  function startEdgeResize(e: MouseEvent, dir: ResizeDirection) {
    if (e.button !== 0) return;
    e.preventDefault();
    void kernel.platform.window?.startResizeDragging(dir);
  }
</script>

<svelte:window onkeydown={onKey} oncontextmenu={onContextMenu} />

{#if pairing}
  <PairBrowser {kernel} />
{:else}
  <div
    class="frame"
    class:resizing
    class:undecorated-outline={Boolean(
      kernel.windowChrome?.borderless && !kernel.windowChrome.tiling && !kernel.windowMaximized && !kernel.windowFullscreen,
    )}
  >
    {#if kernel.showCustomResizeHandles}
      {#each RESIZE_EDGES as edge (edge.dir)}
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="win-edge {edge.cls}" aria-hidden="true" onmousedown={(e) => startEdgeResize(e, edge.dir)}></div>
      {/each}
    {/if}
    <TopBar {kernel} />
    <div class="workspace">
      {#if kernel.leftSidebarOpen}
        <aside class="dock left" aria-label="Workspace sidebar">
          <Sidebar {kernel} />
        </aside>
        <div
          class="resizer"
          role="slider"
          tabindex="0"
          aria-orientation="vertical"
          aria-label="Resize left sidebar"
          aria-valuemin={LEFT_MIN}
          aria-valuemax={LEFT_MAX}
          aria-valuenow={kernel.leftSidebarWidth}
          title="Drag to resize left sidebar · Double-click to reset"
          onpointerdown={(e) => dragSidebar(e, 'left')}
          ondblclick={() => kernel.setLeftSidebarWidth(256)}
          onkeydown={(e) => onSidebarSepKey(e, 'left')}
        ></div>
      {/if}

      <main>
        {#if kernel.platform.safeMode}
          <div class="banner" role="status">Safe mode: no UI plugins are loaded. Remove a broken plugin in Manage plugins, then restart.</div>
        {/if}
        {#if mainView.kind !== 'tab' && !isOverlay}
          <TabBar {kernel} />
        {/if}
        <div class="content">
          {#if mainView.kind === 'core-view'}
            {#key mainView.view}
              <CoreView {kernel} view={mainView.view} />
            {/key}
          {:else if mainView.kind === 'view' && pluginView}
            <SlotHost
              plugin={pluginView.plugin}
              contribution={pluginView.contribution}
              props={{ args: mainView.args }}
              slots={kernel.slots}
              label={pluginView.contribution.title}
            />
          {:else}
            <SplitHost {kernel} node={kernel.layoutRoot} />
          {/if}
        </div>

        {#if mainView.kind === 'settings'}
          <OverlaySheet
            title="Settings"
            subtitle={`Client & ${kernel.connection.label}`}
            hasNestedDialog={Boolean(kernel.dialog)}
            onclose={() => (kernel.main = { kind: 'tab' })}
          >
            <SettingsView {kernel} page={mainView.page} />
          </OverlaySheet>
        {:else if mainView.kind === 'plugins'}
          <OverlaySheet
            title={mainView.starter ? 'Starter pack' : 'Plugins'}
            subtitle="Core plugins, UI plugins & registries"
            hasNestedDialog={Boolean(kernel.dialog)}
            onclose={() => (kernel.main = { kind: 'tab' })}
          >
            <PluginsView {kernel} starter={mainView.starter ?? false} />
          </OverlaySheet>
        {:else if mainView.kind === 'cores'}
          <OverlaySheet
            title="Cores"
            subtitle="Local & remote cores over TLS or SSH"
            hasNestedDialog={Boolean(kernel.dialog)}
            onclose={() => (kernel.main = { kind: 'tab' })}
          >
            <CoresView {kernel} />
          </OverlaySheet>
        {/if}
      </main>

      {#if kernel.rightSidebarOpen}
        <div
          class="resizer"
          role="slider"
          tabindex="0"
          aria-orientation="vertical"
          aria-label="Resize right inspector"
          aria-valuemin={RIGHT_MIN}
          aria-valuemax={RIGHT_MAX}
          aria-valuenow={kernel.rightSidebarWidth}
          title="Drag to resize right inspector · Double-click to reset"
          onpointerdown={(e) => dragSidebar(e, 'right')}
          ondblclick={() => kernel.setRightSidebarWidth(340)}
          onkeydown={(e) => onSidebarSepKey(e, 'right')}
        ></div>
        <aside class="dock right" aria-label="Inspector">
          <RightSidebar {kernel} />
        </aside>
      {/if}
    </div>
    <StatusBar {kernel} />
  </div>
{/if}

{#if kernel.dialog?.kind === 'palette'}
  <Palette {kernel} />
{:else if kernel.dialog?.kind === 'open-folder'}
  <OpenFolder {kernel} connection={kernel.dialog.connection} />
{:else if kernel.dialog?.kind === 'rename-thread'}
  <RenameThread {kernel} address={kernel.dialog.address} title={kernel.dialog.title} />
{:else if kernel.dialog?.kind === 'confirm'}
  <ConfirmDialog
    {kernel}
    title={kernel.dialog.title}
    message={kernel.dialog.message}
    confirmLabel={kernel.dialog.confirmLabel}
    tone={kernel.dialog.tone ?? 'danger'}
    onConfirm={kernel.dialog.onConfirm}
  />
{/if}
<Toasts {kernel} />

<style>
  .frame {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    box-sizing: border-box;
  }
  .frame.undecorated-outline {
    border: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .win-edge {
    position: fixed;
    z-index: 60;
  }
  .edge-n {
    top: 0;
    left: 8px;
    right: 8px;
    height: 4px;
    cursor: n-resize;
  }
  .edge-s {
    bottom: 0;
    left: 8px;
    right: 8px;
    height: 4px;
    cursor: s-resize;
  }
  .edge-w {
    left: 0;
    top: 8px;
    bottom: 8px;
    width: 4px;
    cursor: w-resize;
  }
  .edge-e {
    right: 0;
    top: 8px;
    bottom: 8px;
    width: 4px;
    cursor: e-resize;
  }
  .edge-nw {
    top: 0;
    left: 0;
    width: 8px;
    height: 8px;
    cursor: nw-resize;
  }
  .edge-ne {
    top: 0;
    right: 0;
    width: 8px;
    height: 8px;
    cursor: ne-resize;
  }
  .edge-sw {
    bottom: 0;
    left: 0;
    width: 8px;
    height: 8px;
    cursor: sw-resize;
  }
  .edge-se {
    bottom: 0;
    right: 0;
    width: 8px;
    height: 8px;
    cursor: se-resize;
  }
  .frame.resizing {
    user-select: none;
    cursor: col-resize;
  }
  .workspace {
    display: flex;
    flex: 1;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }
  .dock {
    flex: none;
    min-height: 0;
    overflow: hidden;
  }
  .dock.left {
    width: var(--kk-left-w, 256px);
  }
  .dock.right {
    width: var(--kk-right-w, 340px);
  }
  .resizer {
    flex: none;
    width: 1px;
    position: relative;
    background: var(--kk-border-subtle, CanvasText);
    cursor: col-resize;
    transition: background var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .resizer::after {
    content: '';
    position: absolute;
    top: 0;
    bottom: 0;
    left: -3px;
    right: -3px;
  }
  .resizer:hover,
  .resizer:focus-visible {
    background: var(--kk-accent, Highlight);
  }
  main {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
    background: var(--kk-surface-panel, Canvas);
  }
  .content {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 5px 12px;
    font-size: var(--kk-font-size-small, small);
    background: var(--kk-state-warning-tint, Mark);
    color: var(--kk-text-primary, MarkText);
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
  }
</style>
