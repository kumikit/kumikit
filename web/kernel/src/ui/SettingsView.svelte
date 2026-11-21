<!-- Settings: the pages UI plugins contribute (`settings.page`). -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import CoreSettings from './CoreSettings.svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel, page }: { kernel: Kernel; page: string | undefined } = $props();
  const pages = $derived(
    [...kernel.slots.in('settings.page')].sort((a, b) => (a.contribution.order ?? 100) - (b.contribution.order ?? 100)),
  );
  /** Core plugins with settings on the active tab's core; page ids are `core:<plugin id>`. */
  const conn = $derived(kernel.connection);
  const corePages = $derived(
    conn.hello?.capabilities.includes('settings') ? conn.plugins.filter((p) => p.contributes.settings) : [],
  );
  const coreCurrent = $derived(
    page?.startsWith('core:') ? corePages.find((p) => `core:${p.id}` === page) : pages.length ? undefined : corePages[0],
  );
  const current = $derived(coreCurrent ? undefined : (pages.find((p) => p.contribution.id === page) ?? pages[0]));
</script>

<div class="settings">
  <nav aria-label="Settings pages">
    <h1>Settings</h1>
    {#each pages as p (p)}
      <button type="button" class:current={p === current} aria-current={p === current ? 'page' : undefined} onclick={() => (kernel.main = { kind: 'settings', page: p.contribution.id })}>
        {p.contribution.title}
      </button>
    {/each}
    {#if corePages.length}
      <h2>On {conn.label}</h2>
      {#each corePages as p (p.id)}
        <button type="button" class:current={p === coreCurrent} aria-current={p === coreCurrent ? 'page' : undefined} onclick={() => (kernel.main = { kind: 'settings', page: `core:${p.id}` })}>
          {p.id}
        </button>
      {/each}
    {/if}
  </nav>
  <div class="page">
    {#if coreCurrent}
      {#key coreCurrent.id}
        <CoreSettings connection={conn} plugin={coreCurrent.id} notify={(m) => kernel.notify(m, 'danger')} />
      {/key}
    {:else if current}
      {#key current}
        <SlotHost plugin={current.plugin} contribution={current.contribution} props={{}} slots={kernel.slots} label={current.contribution.title} />
      {/key}
    {:else}
      <div class="kk-empty">
        <p>No settings pages are installed. Themes can still be switched from the command palette.</p>
      </div>
    {/if}
  </div>
</div>

<style>
  .settings {
    display: grid;
    grid-template-columns: 200px 1fr;
    height: 100%;
    min-height: 0;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 12px 8px;
    border-right: 1px solid var(--kk-border-subtle, CanvasText);
    background: var(--kk-surface-panel, Canvas);
  }
  h2 {
    margin: 12px 0 4px 6px;
    font-size: var(--kk-font-size-small, small);
  }
  h1 {
    margin: 0 0 8px 6px;
    font-size: var(--kk-font-size-large, large);
  }
  nav button {
    padding: 6px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: none;
    text-align: left;
    cursor: pointer;
  }
  .current {
    background: var(--kk-surface-selected, Highlight);
  }
  .page {
    overflow: auto;
    padding: 16px 24px;
  }
</style>
