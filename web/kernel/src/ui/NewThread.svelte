<!-- Starting a thread: a provider and model from the core (provider.list/models). -->
<script lang="ts">
  import type { ModelInfo } from '@kumikit/protocol';
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import { trapFocus } from './dialog';

  let { kernel }: { kernel: Kernel } = $props();
  const providers = $derived(kernel.connection.providers);
  let provider = $state('');
  let model = $state('');
  let title = $state('');
  let models = $state.raw<ModelInfo[]>([]);
  let busy = $state(false);

  $effect(() => {
    if (!provider && providers[0]) provider = providers[0].id;
  });
  $effect(() => {
    const p = provider;
    models = [];
    model = '';
    if (!p) return;
    kernel.connection.models(p).then(
      (m) => {
        if (provider !== p) return;
        models = m;
        model = m[0]?.id ?? '';
      },
      () => {},
    );
  });

  function close() {
    kernel.dialog = undefined;
  }

  async function create(e: Event) {
    e.preventDefault();
    if (!provider || busy) return;
    busy = true;
    const t = await kernel.createThread(provider, model || undefined, title.trim() || undefined);
    busy = false;
    if (t) close();
  }

  function onKey(e: KeyboardEvent) {
    if (!isComposing(e) && e.key === 'Escape') close();
  }
</script>

<div class="scrim" role="presentation" onclick={close}></div>
<div class="dialog" role="dialog" aria-modal="true" aria-labelledby="kk-new-title" tabindex="-1" onkeydown={onKey} use:trapFocus>
<form onsubmit={create}>
  <h2 id="kk-new-title">New thread</h2>
  {#if providers.length === 0}
    <p class="kk-muted">No agent provider is installed on this core. Install one from Manage plugins.</p>
    <div class="actions">
      <button type="button" class="kk-button" onclick={() => { close(); kernel.main = { kind: 'plugins' }; }}>Manage plugins</button>
      <button type="button" class="kk-button" onclick={close}>Close</button>
    </div>
  {:else}
    <label>Agent
      <select class="kk-input" bind:value={provider} data-autofocus>
        {#each providers as p (p.id)}<option value={p.id}>{p.name}</option>{/each}
      </select>
    </label>
    <label>Model
      <select class="kk-input" bind:value={model} disabled={models.length === 0}>
        {#each models as m (m.id)}<option value={m.id}>{m.label}</option>{/each}
      </select>
    </label>
    <label>Title <span class="kk-muted">(optional)</span>
      <input class="kk-input" bind:value={title} />
    </label>
    <div class="actions">
      <button type="button" class="kk-button" onclick={close}>Cancel</button>
      <button type="submit" class="kk-button primary">Start</button>
    </div>
  {/if}
</form>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  form {
    display: grid;
    gap: 12px;
  }
  .dialog {
    position: fixed;
    top: 15vh;
    left: 50%;
    display: grid;
    gap: 12px;
    width: min(440px, 92vw);
    transform: translateX(-50%);
    padding: 16px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-overlay, none);
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
