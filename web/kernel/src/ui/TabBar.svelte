<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import type { TabSpec } from '../settings/settings.svelte';

  let { kernel }: { kernel: Kernel } = $props();
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
    const n = kernel.tabs.length;
    if (e.key === 'ArrowRight') kernel.selectTab((i + 1) % n);
    else if (e.key === 'ArrowLeft') kernel.selectTab((i - 1 + n) % n);
    else return;
    e.preventDefault();
  }
</script>

<nav class="tabs" aria-label="Open workspaces">
  {#each kernel.tabs as tab, i (`${tab.connection}/${tab.workspace}`)}
    {@const active = i === kernel.activeIndex && kernel.main.kind === 'tab'}
    {@const st = state(tab)}
    {@const label = core(tab)}
    <div class="tab" class:active data-core={tab.connection}>
      <button type="button" aria-current={active ? 'page' : undefined} onclick={() => kernel.selectTab(i)} onkeydown={(e) => onKey(e, i)}>
        {#if label}<span class="core">{label} ·</span>{/if}
        {name(tab)}
        {#if st}<span class="state" data-state={st}>({st})</span>{/if}
      </button>
      <button type="button" class="close" aria-label={`Close ${name(tab)}`} onclick={() => kernel.closeTab(i)}>×</button>
    </div>
  {/each}
</nav>

<style>
  .tabs {
    display: flex;
    gap: 2px;
    overflow-x: auto;
    padding: 4px 8px 0;
    background: var(--kk-surface-frame, Canvas);
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .tab {
    display: flex;
    align-items: center;
    border: 1px solid transparent;
    border-bottom: 0;
    border-radius: var(--kk-radius-small, 0) var(--kk-radius-small, 0) 0 0;
  }
  .tab.active {
    background: var(--kk-surface-panel, Canvas);
    border-color: var(--kk-border-subtle, CanvasText);
  }
  .tab button {
    padding: 6px 10px;
    border: 0;
    background: none;
    cursor: pointer;
  }
  .close {
    color: var(--kk-text-muted, GrayText);
  }
  .core,
  .state {
    color: var(--kk-text-secondary, CanvasText);
  }
</style>
