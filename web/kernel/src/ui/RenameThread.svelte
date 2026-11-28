<!-- Dialog to rename a thread. The new title is the user's own, so agent suggestions no
     longer replace it (ADR 0030). -->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import type { ThreadAddress } from '@kumikit/ui-sdk';
  import { untrack } from 'svelte';
  import type { Kernel } from '../app.svelte';
  import { trapFocus } from './dialog';

  let {
    kernel,
    address,
    title,
  }: {
    kernel: Kernel;
    address: ThreadAddress;
    title: string;
  } = $props();

  /** Titles are kept within the 200-byte limit the agent tool uses (ADR 0030). */
  const MAX_BYTES = 200;

  let value = $state(untrack(() => title));
  let busy = $state(false);
  const trimmed = $derived(value.trim());
  const bytes = $derived(new TextEncoder().encode(trimmed).length);
  const problem = $derived(
    trimmed === '' ? 'Enter a title.' : bytes > MAX_BYTES ? `Keep the title under ${MAX_BYTES} bytes.` : undefined,
  );
  const canSave = $derived(!busy && problem === undefined && trimmed !== title.trim());

  function close() {
    if (busy) return;
    kernel.dialog = undefined;
  }

  async function save() {
    if (!canSave) return;
    busy = true;
    try {
      const saved = await kernel.saveThreadTitle(address, trimmed);
      if (saved) kernel.dialog = undefined;
    } finally {
      busy = false;
    }
  }

  function onKey(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (e.key === 'Escape') {
      e.stopPropagation();
      close();
    }
  }

  function onInputKey(e: KeyboardEvent) {
    if (isComposing(e) && e.key === 'Enter') {
      e.preventDefault();
    }
  }

  function onSubmit(e: SubmitEvent) {
    e.preventDefault();
    void save();
  }
</script>

<div class="scrim" role="presentation" onclick={close}></div>
<div
  class="dialog"
  role="dialog"
  aria-modal="true"
  aria-labelledby="kk-rename-title"
  aria-describedby="kk-rename-problem"
  tabindex="-1"
  onkeydown={onKey}
  use:trapFocus
>
  <h2 id="kk-rename-title">Rename thread</h2>
  <form class="form" onsubmit={onSubmit}>
    <label class="field">
      <span class="kk-sr-only">Thread title</span>
      <input
        class="kk-input"
        type="text"
        data-autofocus
        bind:value
        disabled={busy}
        aria-invalid={problem !== undefined}
        aria-describedby="kk-rename-problem"
        onkeydown={onInputKey}
        onfocus={(e) => (e.currentTarget as HTMLInputElement).select()}
      />
    </label>
    <p id="kk-rename-problem" class="problem" aria-live="polite">{problem ?? ''}</p>
    <div class="actions">
      <button type="button" class="kk-button" disabled={busy} onclick={close}>Cancel</button>
      <button type="submit" class="kk-button primary" disabled={!canSave}>
        {busy ? 'Saving…' : 'Rename'}
      </button>
    </div>
  </form>
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
  .form {
    display: contents;
  }
  .field {
    display: block;
  }
  .field input {
    width: 100%;
    box-sizing: border-box;
  }
  .problem {
    min-height: 1.2em;
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-state-danger, CanvasText);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 4px;
  }
  .primary:not(:disabled) {
    font-weight: var(--kk-font-weight-strong, bold);
  }
</style>
