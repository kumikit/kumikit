<!-- Opening a project folder on a core on another machine: the folder picker shows this
computer's files, so the path on that machine is typed. -->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import { trapFocus } from './dialog';

  let { kernel, connection }: { kernel: Kernel; connection: string } = $props();
  const conn = $derived(kernel.connectionFor(connection));
  let path = $state('');

  function close() {
    kernel.dialog = undefined;
  }

  async function open(e: Event) {
    e.preventDefault();
    const p = path.trim();
    // Read the props before closing: the dialog unmounts with it.
    const [k, c] = [kernel, connection];
    if (!p) return;
    close();
    await k.openWorkspaceFolder(c, p);
  }

  function onKey(e: KeyboardEvent) {
    if (!isComposing(e) && e.key === 'Escape') close();
  }
</script>

<div class="scrim" role="presentation" onclick={close}></div>
<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-open-title" tabindex="-1" onkeydown={onKey} use:trapFocus>
  <form onsubmit={open}>
    <h2 id="kk-open-title">Open a project folder on {conn?.label ?? connection}</h2>
    <label>Path on {conn?.hello?.core.name ?? 'that machine'}
      <input class="kk-input" bind:value={path} required data-autofocus autocomplete="off" spellcheck="false" placeholder="/home/you/project" />
    </label>
    <div class="actions">
      <button type="button" class="kk-button" onclick={close}>Cancel</button>
      <button type="submit" class="kk-button primary">Open</button>
    </div>
  </form>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 40;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  .dialog {
    position: fixed;
    top: 15vh;
    left: 50%;
    z-index: 41;
    width: min(480px, 92vw);
    transform: translateX(-50%);
    padding: 16px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-overlay, none);
  }
  form {
    display: grid;
    gap: 12px;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
  }
  label {
    display: grid;
    gap: 4px;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
</style>
