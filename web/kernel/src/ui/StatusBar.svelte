<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel }: { kernel: Kernel } = $props();
  const conn = $derived(kernel.connection);
  const label = $derived(
    conn.status === 'ready'
      ? 'Connected'
      : conn.status === 'mismatch'
        ? 'Core version differs'
        : conn.status === 'reconnecting'
          ? 'Reconnecting…'
          : 'Connecting…',
  );
</script>

<footer class="bar">
  <span class="conn" class:down={conn.status !== 'ready'} role="status" aria-live="polite">{label}</span>
  <span class="spacer"></span>
  {#each kernel.slots.in('statusbar.item') as entry (entry)}
    <SlotHost plugin={entry.plugin} contribution={entry.contribution} props={{}} slots={kernel.slots} />
  {/each}
  <button type="button" class="link" onclick={() => (kernel.dialog = { kind: 'palette' })}>
    Commands <kbd>{kernel.mac ? '⌘' : 'Ctrl'}+K</kbd>
  </button>
</footer>

<style>
  .bar {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 2px 10px;
    font-size: var(--kk-font-size-small, small);
    background: var(--kk-surface-frame, Canvas);
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
    color: var(--kk-text-secondary, CanvasText);
  }
  .down {
    color: var(--kk-state-warning, CanvasText);
  }
  .spacer {
    flex: 1;
  }
  .link {
    border: 0;
    background: none;
    cursor: pointer;
    color: inherit;
  }
</style>
