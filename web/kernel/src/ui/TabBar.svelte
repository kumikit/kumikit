<script lang="ts">
  import type { Kernel } from '../app.svelte';

  let { kernel }: { kernel: Kernel } = $props();
  const name = (ws: string) => kernel.connection.workspaces.find((w) => w.id === ws)?.name ?? ws;

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
    <div class="tab" class:active>
      <button type="button" aria-current={active ? 'page' : undefined} onclick={() => kernel.selectTab(i)} onkeydown={(e) => onKey(e, i)}>
        {name(tab.workspace)}
      </button>
      <button type="button" class="close" aria-label={`Close ${name(tab.workspace)}`} onclick={() => kernel.closeTab(i)}>×</button>
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
</style>
