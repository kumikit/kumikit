<script lang="ts">
  import type { Kernel } from '../app.svelte';
  let { kernel }: { kernel: Kernel } = $props();
</script>

<div class="toasts" role="status" aria-live="polite">
  {#each kernel.toasts as t (t.id)}
    <div class="toast {t.tone}">
      <span>{t.message}</span>
      <button type="button" class="close" aria-label="Dismiss" onclick={() => kernel.dismiss(t.id)}>×</button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 16px;
    bottom: 32px;
    display: grid;
    gap: 8px;
    max-width: min(420px, 90vw);
  }
  .toast {
    display: flex;
    gap: 8px;
    align-items: start;
    padding: 8px 12px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-left-width: 4px;
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-raised, none);
  }
  .toast.danger {
    border-left-color: var(--kk-state-danger, CanvasText);
  }
  .toast.warning {
    border-left-color: var(--kk-state-warning, CanvasText);
  }
  .toast.success {
    border-left-color: var(--kk-state-success, CanvasText);
  }
  .close {
    margin-left: auto;
    border: 0;
    background: none;
    cursor: pointer;
  }
</style>
