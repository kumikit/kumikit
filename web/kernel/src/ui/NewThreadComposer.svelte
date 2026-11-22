<!-- Inline new-thread composer: shown directly inside a workspace pane whenever no
     thread is open (including new splits and New Thread), with Agent and Model selectors
     underneath the composer instead of a blocking modal dialog. -->
<script lang="ts">
  import type { ModelInfo } from '@kumikit/protocol';
  import { isComposing } from '@kumikit/ui-sdk';
  import { untrack } from 'svelte';
  import type { Kernel } from '../app.svelte';
  import type { CoreConnection } from '../connections/connection.svelte';

  let {
    kernel,
    connection,
    workspaceId,
    workspaceName,
    paneId,
  }: {
    kernel: Kernel;
    connection: CoreConnection;
    workspaceId: string;
    workspaceName: string | undefined;
    paneId: string;
  } = $props();

  const providers = $derived(connection.providers);
  let provider = $state(untrack(() => kernel.settings.value.lastProvider ?? ''));
  let model = $state(untrack(() => kernel.settings.value.lastModel ?? ''));
  let models = $state.raw<ModelInfo[]>([]);
  let text = $state('');
  let busy = $state(false);
  let textarea: HTMLTextAreaElement | undefined = $state();

  const inputId =
    typeof document !== 'undefined' && document.getElementById('kk-chat-input')
      ? `kk-draft-input-${untrack(() => paneId)}`
      : 'kk-chat-input';

  // Keep a valid provider selected once providers arrive from the core.
  $effect(() => {
    if (providers.length === 0) {
      provider = '';
      return;
    }
    if (!providers.some((p) => p.id === provider)) {
      const remembered = kernel.settings.value.lastProvider;
      provider = (remembered && providers.some((p) => p.id === remembered) ? remembered : providers[0]?.id) ?? '';
    }
  });

  // Load models whenever the selected provider changes.
  $effect(() => {
    const p = provider;
    models = [];
    if (!p) {
      model = '';
      return;
    }
    connection.models(p).then(
      (m) => {
        if (provider !== p) return;
        models = m;
        const remembered = kernel.settings.value.lastModel;
        model = (remembered && m.some((x) => x.id === remembered) ? remembered : m[0]?.id) ?? '';
      },
      () => {},
    );
  });

  // Focus the composer when the user explicitly starts a new draft thread in this pane.
  $effect(() => {
    void kernel.draftFocusNonce;
    if (kernel.activePaneId === paneId && textarea) {
      textarea.focus();
    }
  });

  const canSend = $derived(!busy && Boolean(provider) && text.trim().length > 0 && connection.ready);

  async function send() {
    const message = text.trim();
    if (!message || !provider || busy) return;
    busy = true;
    const created = await kernel.startThreadWithMessage(
      connection.id,
      workspaceId,
      provider,
      model || undefined,
      message,
      paneId,
    );
    busy = false;
    if (created) text = '';
  }

  function onKey(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }
</script>

<section class="draft" aria-label={`New thread in ${workspaceName ?? 'workspace'}`}>
  <div class="inner">
    <div class="intro">
      <h1>{workspaceName ?? 'Workspace'}</h1>
      <p class="kk-muted">Start a new thread — pick an agent and model below, then type your prompt.</p>
    </div>

    <form
      class="composer"
      onsubmit={(e) => {
        e.preventDefault();
        void send();
      }}
    >
      <label class="kk-sr-only" for={inputId}>Message</label>
      <textarea
        bind:this={textarea}
        id={inputId}
        rows="3"
        disabled={providers.length === 0 || busy}
        placeholder={providers.length === 0 ? 'Install an agent provider plugin to start a thread' : 'Message the agent…'}
        bind:value={text}
        onkeydown={onKey}
      ></textarea>
      <div class="bar">
        {#if providers.length === 0}
          <span class="no-provider">No agent provider is installed on this core.</span>
          <button type="button" class="kk-button" onclick={() => (kernel.main = { kind: 'plugins' })}>Manage plugins</button>
        {:else}
          <div class="selectors">
            <select
              class="picker"
              aria-label="Agent"
              title="Agent provider"
              bind:value={provider}
              disabled={busy}
            >
              {#each providers as p (p.id)}
                <option value={p.id}>{p.name}</option>
              {/each}
            </select>
            <select
              class="picker"
              aria-label="Model"
              title="Model"
              bind:value={model}
              disabled={models.length === 0 || busy}
            >
              {#each models as m (m.id)}
                <option value={m.id}>{m.label}</option>
              {/each}
            </select>
          </div>
          <button
            type="submit"
            class="send-btn"
            disabled={!canSend}
            aria-label="Send"
            title="Start thread and send (Enter)"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              stroke-width="1.8"
              stroke-linecap="round"
              stroke-linejoin="round"
              aria-hidden="true"
            >
              <line x1="8" y1="12.5" x2="8" y2="3.5" />
              <polyline points="4 7.25 8 3.25 12 7.25" />
            </svg>
          </button>
        {/if}
      </div>
    </form>
  </div>
</section>

<style>
  .draft {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 24px 20px;
    overflow: auto;
  }
  .inner {
    width: 100%;
    max-width: 720px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }
  .intro {
    text-align: center;
    display: grid;
    gap: 4px;
  }
  h1 {
    margin: 0;
    font-size: var(--kk-font-size-title, x-large);
    color: var(--kk-text-primary, CanvasText);
  }
  .intro p {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
  }
  .composer {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 14px 10px;
    border: 1px solid var(--kk-border-subtle, FieldText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-input, Field);
    box-shadow: var(--kk-shadow-card, none);
    transition: border-color var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .composer:focus-within {
    border-color: var(--kk-border-strong, Highlight);
  }
  textarea {
    width: 100%;
    min-height: 60px;
    max-height: 220px;
    resize: vertical;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--kk-text-primary, FieldText);
    font: inherit;
    line-height: var(--kk-line-height-base, normal);
  }
  textarea:focus {
    outline: none;
  }
  .bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    padding-top: 6px;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .selectors {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 6px;
    min-width: 0;
  }
  .picker {
    height: 26px;
    max-width: 220px;
    padding: 0 8px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-card, Field);
    color: var(--kk-text-primary, FieldText);
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .picker:hover:not(:disabled) {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .picker:disabled {
    opacity: 0.55;
    cursor: default;
  }
  .no-provider {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .send-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 0;
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-accent, Highlight);
    color: var(--kk-text-on-accent, HighlightText);
    cursor: pointer;
    flex: none;
    transition:
      background var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out),
      opacity var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .send-btn:hover:not(:disabled) {
    background: var(--kk-accent-hover, Highlight);
  }
  .send-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
</style>
