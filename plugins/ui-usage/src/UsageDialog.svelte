<!-- Agent usage: an Overview tab and one tab per provider enabled on the core, each with
     its plan limits (as the provider reported them) and the usage of the turns this core
     ran. Opens on `initial`. Refresh asks providers that can read limits to read them now;
     nothing refreshes on a timer. -->
<script lang="ts">
  import type { PluginContext } from '@kumikit/ui-sdk';
  import { onMount } from 'svelte';

  import Overview from './Overview.svelte';
  import ProviderPanel from './ProviderPanel.svelte';
  import type { UsageStore } from './store.svelte';

  let { ctx, store, initial, onclose }: { ctx: PluginContext; store: UsageStore; initial?: string | undefined; onclose: () => void } =
    $props();

  let dialog: HTMLDialogElement | undefined = $state();
  // svelte-ignore state_referenced_locally
  let tab = $state<string>(initial ?? 'overview');
  const titleId = `kk-usage-${Math.random().toString(36).slice(2)}`;

  const tabs = $derived([{ id: 'overview', name: 'Overview', icon: undefined as string | undefined }, ...store.providers.map((p) => ({ id: p.id, name: p.name, icon: p.icon }))]);
  const current = $derived(store.providers.find((p) => p.id === tab));
  const readers = $derived(store.providers.filter((p) => p.readsLimits));
  const busy = $derived(Object.keys(store.refreshing).length > 0);
  const canRefresh = $derived(tab === 'overview' ? readers.length > 0 : current?.readsLimits === true);

  onMount(() => {
    const d = dialog;
    if (!d) return;
    // jsdom and old engines lack showModal; an open dialog still works there.
    if (typeof d.showModal === 'function') d.showModal();
    else d.setAttribute('open', '');
    void store.load().then(() => {
      if (tab !== 'overview' && !store.providers.some((p) => p.id === tab)) tab = 'overview';
    }, (e: unknown) => ctx.notify(`Cannot read usage: ${message(e)}`, 'danger'));
    return () => {
      if (d.open && typeof d.close === 'function') d.close();
    };
  });

  function message(e: unknown): string {
    return e instanceof Error ? e.message : String(e);
  }

  function close() {
    if (dialog?.open && typeof dialog.close === 'function') dialog.close();
    else onclose();
  }

  async function refresh() {
    const ids = tab === 'overview' ? readers.map((p) => p.id) : current?.readsLimits ? [current.id] : [];
    const results = await Promise.allSettled(ids.map((id) => store.refresh(id)));
    results.forEach((r, i) => {
      if (r.status === 'rejected') {
        const name = store.providers.find((p) => p.id === ids[i])?.name ?? ids[i];
        ctx.notify(`${name}: cannot read plan limits: ${message(r.reason)}`, 'warning');
      }
    });
    store.version += 1;
  }

  function onTabKey(e: KeyboardEvent) {
    const i = tabs.findIndex((t) => t.id === tab);
    let next = i;
    if (e.key === 'ArrowRight') next = (i + 1) % tabs.length;
    else if (e.key === 'ArrowLeft') next = (i - 1 + tabs.length) % tabs.length;
    else if (e.key === 'Home') next = 0;
    else if (e.key === 'End') next = tabs.length - 1;
    else return;
    e.preventDefault();
    tab = tabs[next]!.id;
    (e.currentTarget as HTMLElement).querySelector<HTMLButtonElement>(`[data-tab="${CSS.escape(tab)}"]`)?.focus();
  }
</script>

<dialog
  bind:this={dialog}
  class="kk-usage"
  aria-labelledby={titleId}
  onclose={onclose}
  onclick={(e) => {
    // A click on the backdrop lands on the dialog element itself.
    if (e.target === dialog) close();
  }}
>
  <div class="frame">
    <header>
      <h2 id={titleId}>Agent usage</h2>
      <button type="button" class="icon-btn" aria-label="Close" title="Close (Escape)" onclick={close}>
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
          <path d="M4 4l8 8M12 4l-8 8" />
        </svg>
      </button>
    </header>
    <div class="tabrow">
      <div class="tabs" role="tablist" aria-label="Providers" tabindex="-1" onkeydown={onTabKey}>
        {#each tabs as t (t.id)}
          <button
            type="button"
            role="tab"
            class="tab"
            id={`${titleId}-tab-${t.id}`}
            data-tab={t.id}
            aria-selected={tab === t.id}
            aria-controls={`${titleId}-panel`}
            tabindex={tab === t.id ? 0 : -1}
            onclick={() => (tab = t.id)}
          >
            {#if t.id === 'overview'}
              <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" aria-hidden="true">
                <path d="M3 13V8M8 13V3M13 13V6" />
              </svg>
            {:else if t.icon}
              <img class="picon" src={t.icon} alt="" aria-hidden="true" draggable="false" />
            {:else}
              <span class="picon badge" aria-hidden="true">{(t.name.trim()[0] ?? '?').toUpperCase()}</span>
            {/if}
            <span>{t.name}</span>
          </button>
        {/each}
      </div>
      <button
        type="button"
        class="icon-btn"
        aria-label={tab === 'overview' ? 'Refresh plan limits of every provider' : `Refresh ${current?.name ?? ''} plan limits`}
        title={canRefresh ? 'Ask the provider to read its plan limits now' : 'This provider reports limits only while it runs'}
        disabled={!canRefresh || busy}
        aria-busy={busy}
        onclick={refresh}
      >
        <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
          <path d="M13 8a5 5 0 1 1-1.5-3.5M13 2.5V5h-2.5" />
        </svg>
      </button>
    </div>
    <div class="panel" id={`${titleId}-panel`} role="tabpanel" aria-labelledby={`${titleId}-tab-${tab}`}>
      {#if tab === 'overview'}
        <Overview {ctx} {store} onopen={(id) => (tab = id)} />
      {:else if current}
        {#key current.id}
          <ProviderPanel {ctx} {store} provider={current} />
        {/key}
      {/if}
    </div>
    <footer>
      Statistics count the turns this core ran. Plan limits are updated while agents run and on demand.
    </footer>
  </div>
</dialog>

<style>
  .kk-usage {
    width: min(760px, calc(100vw - 48px));
    max-height: min(860px, calc(100vh - 48px));
    max-width: none;
    padding: 0;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-primary, CanvasText);
    box-shadow: var(--kk-shadow-overlay, none);
    font-family: var(--kk-font-ui, sans-serif);
    font-size: var(--kk-font-size-base, medium);
  }
  .kk-usage::backdrop {
    background: color-mix(in srgb, var(--kk-surface-overlay, Canvas) calc(var(--kk-overlay-opacity, 0.6) * 100%), transparent);
    backdrop-filter: blur(var(--kk-overlay-blur, 0px));
  }
  .frame {
    display: flex;
    flex-direction: column;
    max-height: inherit;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    padding: 16px 18px 10px;
  }
  h2 {
    flex: 1;
    margin: 0;
    font-size: var(--kk-font-size-title, x-large);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .tabrow {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 0 18px 12px;
  }
  .tabs {
    flex: 1;
    min-width: 0;
    display: flex;
    gap: 2px;
    padding: 3px;
    overflow-x: auto;
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  .tab {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex: none;
    padding: 5px 10px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-secondary, ButtonText);
    font: inherit;
    cursor: pointer;
  }
  .tab:hover {
    color: var(--kk-text-primary, ButtonText);
  }
  .tab[aria-selected='true'] {
    background: var(--kk-surface-selected, Highlight);
    color: var(--kk-text-primary, HighlightText);
  }
  .tab:focus-visible,
  .icon-btn:focus-visible {
    outline: 2px solid var(--kk-focus, Highlight);
    outline-offset: 1px;
  }
  .picon {
    width: 14px;
    height: 14px;
    object-fit: contain;
  }
  .picon.badge {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-hover, ButtonFace);
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .icon-btn {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    flex: none;
    width: 28px;
    height: 28px;
    padding: 0;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-medium, 0);
    background: transparent;
    color: var(--kk-text-secondary, ButtonText);
    cursor: pointer;
  }
  .icon-btn:hover:not(:disabled) {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, ButtonText);
  }
  .icon-btn:disabled {
    cursor: default;
    opacity: 0.5;
  }
  .panel {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 4px 18px 8px;
  }
  footer {
    padding: 10px 18px 14px;
    color: var(--kk-text-muted, GrayText);
    font-size: var(--kk-font-size-small, small);
    text-align: center;
  }
</style>
