<!-- Settings inside the overlay sheet: client pages (`settings.page`) and core plugin
     settings (`settings.schema`), arranged in a clean two-column settings layout. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import { isAgentInfo } from '../plugins/agents';
  import AgentsSettings from './AgentsSettings.svelte';
  import CoreSettings from './CoreSettings.svelte';
  import GeneralSettings from './GeneralSettings.svelte';
  import SlotHost from './SlotHost.svelte';
  import AboutSettings from './AboutSettings.svelte';
  import WorktreesSettings from './WorktreesSettings.svelte';

  let { kernel, page }: { kernel: Kernel; page: string | undefined } = $props();
  const pages = $derived(
    [...kernel.slots.in('settings.page')].sort((a, b) => (a.contribution.order ?? 100) - (b.contribution.order ?? 100)),
  );
  const showGeneral = $derived(!kernel.platform.safeMode);
  /** Core plugins with settings on the active tab's core; page ids are `core:<plugin id>`.
   * Agents' settings are on the Agents page instead. */
  const conn = $derived(kernel.connection);
  const corePages = $derived(
    conn.hello?.capabilities.includes('settings') ? conn.plugins.filter((p) => p.contributes.settings && !isAgentInfo(p)) : [],
  );
  /** The Agents page manages the core's plugins, so it needs the `plugins` capability. */
  const showAgents = $derived(conn.hello?.capabilities.includes('plugins') ?? false);
  const agentsCurrent = $derived(showAgents && page === 'agents');
  const generalCurrent = $derived(showGeneral && (page === undefined || page === 'general'));
  /** About took over Updates; `updates` still opens it (the status bar links there). */
  const aboutCurrent = $derived(showGeneral && (page === 'about' || page === 'updates'));
  const worktreesCurrent = $derived(showGeneral && page === 'worktrees');
  const coreCurrent = $derived(
    generalCurrent || aboutCurrent || worktreesCurrent || agentsCurrent
      ? undefined
      : page?.startsWith('core:')
        ? corePages.find((p) => `core:${p.id}` === page)
        : pages.length
          ? undefined
          : corePages[0],
  );
  const current = $derived(
    generalCurrent || aboutCurrent || worktreesCurrent || agentsCurrent || coreCurrent
      ? undefined
      : (pages.find((p) => p.contribution.id === page) ?? pages[0]),
  );
</script>

<div class="settings">
  <nav aria-label="Settings pages">
    {#if showGeneral || pages.length}
      <span class="group-label">Client</span>
      {#if showGeneral}
        <button
          type="button"
          class:current={generalCurrent}
          aria-current={generalCurrent ? 'page' : undefined}
          onclick={() => (kernel.main = { kind: 'settings', page: 'general' })}
        >
          General
        </button>
        <button
          type="button"
          class:current={worktreesCurrent}
          aria-current={worktreesCurrent ? 'page' : undefined}
          onclick={() => (kernel.main = { kind: 'settings', page: 'worktrees' })}
        >
          Worktrees
        </button>
      {/if}
      {#each pages as p (p)}
        <button
          type="button"
          class:current={p === current}
          aria-current={p === current ? 'page' : undefined}
          onclick={() => (kernel.main = { kind: 'settings', page: p.contribution.id })}
        >
          {p.contribution.title}
        </button>
      {/each}
      {#if showGeneral}
        <button
          type="button"
          class="nav-with-dot"
          class:current={aboutCurrent}
          aria-current={aboutCurrent ? 'page' : undefined}
          onclick={() => (kernel.main = { kind: 'settings', page: 'about' })}
        >
          <span>About</span>
          {#if kernel.updates.hasAttention}
            <span class="update-dot" aria-label="Update available"></span>
          {/if}
        </button>
      {/if}
    {/if}
    {#if showAgents || corePages.length}
      <span class="group-label">On {conn.label}</span>
      {#if showAgents}
        <button
          type="button"
          class:current={agentsCurrent}
          aria-current={agentsCurrent ? 'page' : undefined}
          onclick={() => (kernel.main = { kind: 'settings', page: 'agents' })}
        >
          Agents
        </button>
      {/if}
      {#each corePages as p (p.id)}
        <button
          type="button"
          class:current={p === coreCurrent}
          aria-current={p === coreCurrent ? 'page' : undefined}
          onclick={() => (kernel.main = { kind: 'settings', page: `core:${p.id}` })}
        >
          {p.id}
        </button>
      {/each}
    {/if}
  </nav>
  <div class="page">
    <div class="page-inner">
      {#if generalCurrent}
        <GeneralSettings {kernel} />
      {:else if aboutCurrent}
        <AboutSettings {kernel} />
      {:else if worktreesCurrent}
        <WorktreesSettings {kernel} />
      {:else if agentsCurrent}
        <AgentsSettings {kernel} />
      {:else if coreCurrent}
        {#key coreCurrent.id}
          <h2 class="core-heading">{coreCurrent.id}</h2>
          <p class="kk-muted core-sub">Plugin settings stored on {conn.label}.</p>
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
</div>

<style>
  .settings {
    display: grid;
    grid-template-columns: 210px 1fr;
    flex: 1;
    min-height: 0;
  }
  nav {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 14px 10px;
    border-right: 1px solid var(--kk-border-subtle, CanvasText);
    background: var(--kk-surface-frame, Canvas);
    overflow: auto;
  }
  .group-label {
    margin: 10px 8px 4px;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-muted, GrayText);
    text-transform: uppercase;
    letter-spacing: 0.04em;
  }
  .group-label:first-child {
    margin-top: 2px;
  }
  nav button {
    padding: 6px 10px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    text-align: left;
    cursor: pointer;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  nav button:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  nav button.current {
    background: var(--kk-surface-selected, Highlight);
    color: var(--kk-text-primary, CanvasText);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .nav-with-dot {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 6px;
  }
  .update-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-accent, Highlight);
    flex: none;
  }
  .page {
    overflow: auto;
    padding: 24px 32px;
    background: var(--kk-surface-panel, Canvas);
  }
  .page-inner {
    max-width: 680px;
  }
  .core-heading {
    margin: 0 0 4px;
    font-size: var(--kk-font-size-large, large);
  }
  .core-sub {
    margin: 0 0 16px;
    font-size: var(--kk-font-size-small, small);
  }
</style>
