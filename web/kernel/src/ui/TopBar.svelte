<!-- Zed-inspired thin top utility bar: sidebar toggles, workspace/thread actions,
     breadcrumb, and split controls. Global actions (plugins, settings, theme, commands)
     live in the bottom-left of the status bar. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import Icon from './Icon.svelte';

  let { kernel }: { kernel: Kernel } = $props();

  const modKey = $derived(kernel.mac ? '⌘' : 'Ctrl+');
  const inspectorTabs = $derived(kernel.slotsIn('inspector.tab'));
  const inWorkspace = $derived(kernel.main.kind === 'tab');
  const activeWsName = $derived.by(() => {
    const t = kernel.activeTab;
    if (!t) return undefined;
    return kernel.connectionFor(t.connection)?.workspaces.find((w) => w.id === t.workspace)?.name;
  });
  const viewLabel = $derived.by(() => {
    const m = kernel.main;
    if (m.kind === 'settings') return 'Settings';
    if (m.kind === 'plugins') return m.starter ? 'Starter pack' : 'Plugins';
    if (m.kind === 'cores') return 'Cores';
    if (m.kind === 'core-view') return m.view.title;
    if (m.kind === 'view') return kernel.slots.in('view').find((v) => v.contribution.id === m.id)?.contribution.title ?? 'View';
    return undefined;
  });
  function onTopBarMouseDown(e: MouseEvent) {
    if (!kernel.canDragTopBar || e.button !== 0) return;
    if (e.target instanceof Element && e.target.closest('button, a, input, select, textarea, [role="button"]')) {
      return;
    }
    if (e.detail === 2) {
      void kernel.platform.window?.toggleMaximize();
    } else {
      void kernel.platform.window?.startDragging();
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<header class="topbar" class:overlay-inset={kernel.nativeOverlayInset} onmousedown={onTopBarMouseDown}>
  <div class="group">
    <button
      type="button"
      class="kk-icon-btn"
      class:active={kernel.leftSidebarOpen}
      aria-label="Toggle left sidebar"
      aria-pressed={kernel.leftSidebarOpen}
      title={`Toggle left sidebar (${modKey}B)`}
      onclick={() => kernel.toggleLeftSidebar()}
    >
      <Icon name="sidebar-left" />
    </button>
    <button
      type="button"
      class="kk-icon-btn"
      aria-label="New chat"
      title={`New chat (${modKey}Shift+N)`}
      disabled={!kernel.connection.ready}
      onclick={() => void kernel.startNewChat()}
    >
      <Icon name="message-plus" />
    </button>
  </div>

  <div class="center">
    {#if !inWorkspace}
      <button
        type="button"
        class="back"
        aria-label="Back to workspace"
        title="Return to workspace panes"
        onclick={() => (kernel.main = { kind: 'tab' })}
      >
        <Icon name="arrow-left" size={13} />
        <span>Workspace</span>
      </button>
      {#if viewLabel}
        <span class="sep" aria-hidden="true">/</span>
        <span class="crumb">{viewLabel}</span>
      {/if}
    {:else if activeWsName}
      <span class="brand">Kumikit</span>
      <span class="sep" aria-hidden="true">/</span>
      <span class="crumb">{activeWsName}</span>
    {:else}
      <span class="brand">Kumikit</span>
    {/if}
  </div>

  <div class="group">
    <button
      type="button"
      class="kk-icon-btn"
      aria-label="Split pane right"
      title={`Split right with a new thread (${modKey}\\)`}
      onclick={() => kernel.splitPane('horizontal')}
    >
      <Icon name="split-h" />
    </button>
    <button
      type="button"
      class="kk-icon-btn"
      aria-label="Split pane down"
      title={`Split down with a new thread (${modKey}Shift+\\)`}
      onclick={() => kernel.splitPane('vertical')}
    >
      <Icon name="split-v" />
    </button>
    <span class="divider" aria-hidden="true"></span>
    <button
      type="button"
      class="kk-icon-btn inspector-toggle"
      class:active={kernel.rightSidebarOpen}
      aria-label="Toggle right inspector"
      aria-pressed={kernel.rightSidebarOpen}
      title={`Toggle right inspector (${modKey}Shift+B)${inspectorTabs.length > 0 ? ` — ${inspectorTabs.map((e) => e.contribution.title).join(', ')}` : ''}`}
      onclick={() => kernel.toggleRightSidebar()}
    >
      <Icon name="sidebar-right" />
      {#if inspectorTabs.length > 0 && !kernel.rightSidebarOpen}
        <span class="dot" aria-hidden="true"></span>
      {/if}
    </button>
    {#if kernel.showWindowButtons}
      <span class="divider" aria-hidden="true"></span>
      <div class="win-controls" role="group" aria-label="Window controls">
        <button
          type="button"
          class="kk-icon-btn win-btn"
          aria-label="Minimize window"
          title="Minimize"
          onclick={() => void kernel.platform.window?.minimize()}
        >
          <Icon name="win-minimize" size={12} />
        </button>
        <button
          type="button"
          class="kk-icon-btn win-btn"
          aria-label={kernel.windowMaximized ? 'Restore window' : 'Maximize window'}
          title={kernel.windowMaximized ? 'Restore' : 'Maximize'}
          onclick={() => void kernel.platform.window?.toggleMaximize()}
        >
          <Icon name={kernel.windowMaximized ? 'win-restore' : 'win-maximize'} size={12} />
        </button>
        <button
          type="button"
          class="kk-icon-btn win-btn close"
          aria-label="Close window"
          title="Close"
          onclick={() => void kernel.platform.window?.close()}
        >
          <Icon name="x" size={12} />
        </button>
      </div>
    {/if}
  </div>
</header>

<style>
  .topbar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    height: 32px;
    padding: 0 6px;
    background: var(--kk-surface-frame, Canvas);
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    user-select: none;
    flex: none;
  }
  .topbar.overlay-inset {
    padding-left: 74px;
  }
  .win-controls {
    display: flex;
    align-items: center;
    gap: 1px;
  }
  .win-btn {
    width: 26px;
    height: 24px;
  }
  .win-btn.close:hover {
    background: var(--kk-state-danger-tint, Mark);
    color: var(--kk-state-danger, MarkText);
  }
  .group {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .center {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .brand {
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
  }
  .crumb {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--kk-text-primary, CanvasText);
  }
  .sep {
    color: var(--kk-text-muted, GrayText);
  }
  .back {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 22px;
    padding: 0 6px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-secondary, CanvasText);
    cursor: pointer;
  }
  .back:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .divider {
    width: 1px;
    height: 14px;
    margin: 0 3px;
    background: var(--kk-border-subtle, CanvasText);
  }
  .inspector-toggle {
    position: relative;
  }
  .dot {
    position: absolute;
    top: 4px;
    right: 3px;
    width: 5px;
    height: 5px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-accent, Highlight);
  }
</style>
