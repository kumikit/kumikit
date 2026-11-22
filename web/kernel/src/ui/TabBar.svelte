<!-- Thin Zed-style per-pane tab bar: workspace tabs with compact close buttons and
     pane-level split/close icon controls when split. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import { clampTabIndex, type LayoutLeaf } from '../layout/layout';
  import type { TabSpec } from '../settings/settings.svelte';
  import Icon from './Icon.svelte';

  let { kernel, pane }: { kernel: Kernel; pane?: LayoutLeaf } = $props();

  const target = $derived(pane ?? kernel.activePane);
  const isFocused = $derived(target.id === kernel.activePaneId);
  const isSplit = $derived(kernel.paneCount > 1);
  const tabs = $derived(target.tabs);
  const activeIdx = $derived(clampTabIndex(tabs, target.activeTab));

  const connText: Record<string, string> = {
    connecting: 'connecting',
    reconnecting: 'reconnecting',
    mismatch: 'different version',
    unauthorized: 'needs pairing',
    certificate_changed: 'certificate changed',
  };
  const name = (tab: TabSpec) => kernel.connectionFor(tab.connection)?.workspaces.find((w) => w.id === tab.workspace)?.name ?? tab.workspace;
  /** The tab's core state, when it is not simply connected. */
  const state = (tab: TabSpec) => {
    const c = kernel.connectionFor(tab.connection);
    if (!c) return 'removed';
    return c.status === 'ready' ? undefined : (connText[c.status] ?? c.status);
  };
  /** Names the core when several are known. */
  const core = (tab: TabSpec) => (kernel.connections.length > 1 ? (kernel.connectionFor(tab.connection)?.label ?? tab.connection) : undefined);

  function onKey(e: KeyboardEvent, i: number) {
    const n = tabs.length;
    if (n === 0) return;
    if (e.key === 'ArrowRight') kernel.selectTab((i + 1) % n, target.id);
    else if (e.key === 'ArrowLeft') kernel.selectTab((i - 1 + n) % n, target.id);
    else return;
    e.preventDefault();
  }
</script>

{#if tabs.length > 0 || isSplit}
  <div class="tabbar">
    <nav class="tabs" aria-label={isFocused ? 'Open workspaces' : `Open workspaces (${target.id})`}>
      {#each tabs as tab, i (`${tab.connection}/${tab.workspace}`)}
        {@const active = i === activeIdx && kernel.main.kind === 'tab' && isFocused}
        {@const selected = i === activeIdx}
        {@const st = state(tab)}
        {@const label = core(tab)}
        <div class="tab" class:active class:selected data-core={tab.connection}>
          <button
            type="button"
            class="tab-btn"
            aria-current={active ? 'page' : undefined}
            onclick={() => kernel.selectTab(i, target.id)}
            onkeydown={(e) => onKey(e, i)}
          >
            {#if label}<span class="core">{label} ·</span>{/if}
            {name(tab)}
            {#if st}<span class="state" data-state={st}>({st})</span>{/if}
          </button>
          <button
            type="button"
            class="close"
            aria-label={`Close ${name(tab)}`}
            title={`Close ${name(tab)}`}
            onclick={() => kernel.closeTab(i, target.id)}
          >
            <Icon name="x" size={11} />
          </button>
        </div>
      {/each}
    </nav>
    {#if isSplit}
      <div class="pane-actions">
        <button
          type="button"
          class="kk-icon-btn"
          aria-label="Split this pane right"
          title="Split this pane right"
          onclick={() => kernel.splitPane('horizontal', target.id)}
        >
          <Icon name="split-h" size={12} />
        </button>
        <button
          type="button"
          class="kk-icon-btn"
          aria-label="Split this pane down"
          title="Split this pane down"
          onclick={() => kernel.splitPane('vertical', target.id)}
        >
          <Icon name="split-v" size={12} />
        </button>
        <button
          type="button"
          class="kk-icon-btn"
          aria-label="Close split pane"
          title="Close split pane"
          onclick={() => kernel.closePane(target.id)}
        >
          <Icon name="x" size={12} />
        </button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .tabbar {
    display: flex;
    align-items: stretch;
    height: 30px;
    background: var(--kk-surface-frame, Canvas);
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    flex: none;
  }
  .tabs {
    display: flex;
    align-items: stretch;
    flex: 1;
    min-width: 0;
    overflow-x: auto;
  }
  .tab {
    display: flex;
    align-items: center;
    border-right: 1px solid var(--kk-border-subtle, CanvasText);
    background: var(--kk-surface-frame, Canvas);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .tab:hover {
    background: var(--kk-surface-hover, Canvas);
    color: var(--kk-text-primary, CanvasText);
  }
  .tab.selected {
    background: var(--kk-surface-panel, Canvas);
    color: var(--kk-text-primary, CanvasText);
  }
  .tab.active {
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .tab-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 100%;
    padding: 0 8px 0 10px;
    border: 0;
    background: transparent;
    cursor: pointer;
    white-space: nowrap;
  }
  .close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 20px;
    margin-right: 4px;
    padding: 0;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-muted, GrayText);
    cursor: pointer;
  }
  .close:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .core,
  .state {
    color: var(--kk-text-secondary, CanvasText);
    font-weight: var(--kk-font-weight-normal, normal);
  }
  .pane-actions {
    display: flex;
    align-items: center;
    gap: 1px;
    padding: 0 4px;
    border-left: 1px solid var(--kk-border-subtle, CanvasText);
    flex: none;
  }
</style>
