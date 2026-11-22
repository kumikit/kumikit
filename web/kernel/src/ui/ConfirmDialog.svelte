<!-- Confirmation modal for destructive actions (such as deleting a thread or project). -->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import { trapFocus } from './dialog';

  let {
    kernel,
    title,
    message,
    confirmLabel,
    tone = 'danger',
    onConfirm,
  }: {
    kernel: Kernel;
    title: string;
    message: string;
    confirmLabel: string;
    tone?: 'danger' | 'primary';
    onConfirm: () => Promise<void> | void;
  } = $props();

  let busy = $state(false);

  function close() {
    if (busy) return;
    kernel.dialog = undefined;
  }

  async function confirm() {
    if (busy) return;
    busy = true;
    try {
      await onConfirm();
    } finally {
      busy = false;
      kernel.dialog = undefined;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (!isComposing(e) && e.key === 'Escape') {
      e.stopPropagation();
      close();
    }
  }
</script>

<div class="scrim" role="presentation" onclick={close}></div>
<div
  class="dialog"
  role="alertdialog"
  aria-modal="true"
  aria-labelledby="kk-confirm-title"
  aria-describedby="kk-confirm-msg"
  tabindex="-1"
  onkeydown={onKey}
  use:trapFocus
>
  <h2 id="kk-confirm-title">{title}</h2>
  <p id="kk-confirm-msg" class="msg">{message}</p>
  <div class="actions">
    <button type="button" class="kk-button" data-autofocus disabled={busy} onclick={close}>
      Cancel
    </button>
    <button
      type="button"
      class="kk-button"
      class:primary={tone === 'primary'}
      class:danger={tone === 'danger'}
      disabled={busy}
      onclick={() => void confirm()}
    >
      {busy ? 'Working…' : confirmLabel}
    </button>
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  .dialog {
    position: fixed;
    top: 20vh;
    left: 50%;
    z-index: 51;
    width: min(440px, 92vw);
    transform: translateX(-50%);
    padding: 18px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-overlay, none);
    display: grid;
    gap: 12px;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
    color: var(--kk-text-primary, CanvasText);
  }
  .msg {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
    line-height: var(--kk-line-height-body, 1.5);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
  .danger {
    color: var(--kk-state-danger, CanvasText);
    border-color: var(--kk-state-danger, CanvasText);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .danger:hover:not(:disabled) {
    background: var(--kk-surface-hover, ButtonFace);
  }
</style>
