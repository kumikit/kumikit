<!-- Resizable right inspector dock: hosts `inspector.tab` UI plugins (including
     sandboxed Level 2 iframe panels like `kumikit.ui-git`) with open closable tabs and an
     icon dropdown menu to open a new plugin tab or focus an existing one. -->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import { untrack } from 'svelte';
  import type { Kernel } from '../app.svelte';
  import type { Entry } from '../slots/registry.svelte';
  import Icon from './Icon.svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel }: { kernel: Kernel } = $props();

  const available = $derived(kernel.slotsIn('inspector.tab'));
  const entryKey = (e: Entry<'inspector.tab'>) => `${e.plugin}/${e.contribution.id}`;

  const openEntries = $derived.by(() => {
    const saved = kernel.openInspectorTabs;
    if (saved === undefined) return available.length > 0 ? [available[0]!] : [];
    const byKey = new Map(available.map((e) => [entryKey(e), e]));
    const byId = new Map(available.map((e) => [e.contribution.id, e]));
    const out: Entry<'inspector.tab'>[] = [];
    const seen = new Set<string>();
    for (const k of saved) {
      const match = byKey.get(k) ?? byId.get(k);
      if (match) {
        const key = entryKey(match);
        if (!seen.has(key)) {
          seen.add(key);
          out.push(match);
        }
      }
    }
    return out;
  });

  const openKeySet = $derived(new Set(openEntries.map(entryKey)));
  const selectedKey = $derived(kernel.activeInspectorTab);
  const active = $derived(
    openEntries.find((e) => entryKey(e) === selectedKey || e.contribution.id === selectedKey) ?? openEntries[0],
  );
  const activeTab = $derived(kernel.activeTab);
  const activeThread = $derived(kernel.activeThread);
  const scopeKey = $derived(
    active
      ? `${activeTab?.connection ?? kernel.connection.id}/${activeTab?.workspace ?? ''}/${activeThread?.thread ?? ''}/${active.plugin}/${active.contribution.id}`
      : '',
  );
  const panelProps = $derived.by(() => {
    void scopeKey;
    return untrack(() => ({
      address: activeThread,
      connection: kernel.pluginConnection(activeTab?.connection),
    }));
  });

  let menuOpen = $state(false);
  let menuWrap: HTMLElement | undefined = $state();
  let triggerBtn: HTMLButtonElement | undefined = $state();

  function openPlugin(key: string) {
    menuOpen = false;
    if (kernel.openInspectorTabs === undefined) {
      const seeded = openEntries.map(entryKey);
      if (!seeded.includes(key)) seeded.push(key);
      kernel.settings.update({ rightSidebarOpen: true, openInspectorTabs: seeded, activeInspectorTab: key });
    } else {
      kernel.openInspectorPlugin(key);
    }
  }

  function onWindowPointerDown(e: PointerEvent) {
    if (!menuOpen) return;
    if (menuWrap && !menuWrap.contains(e.target as Node)) {
      menuOpen = false;
    }
  }

  function onWindowKeyDown(e: KeyboardEvent) {
    if (!menuOpen || isComposing(e)) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      menuOpen = false;
      triggerBtn?.focus();
    }
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydown={onWindowKeyDown} />

<div class="inspector">
  <div class="head">
    <div class="tabs" role="group" aria-label="Inspector panels">
      {#each openEntries as entry (entryKey(entry))}
        {@const key = entryKey(entry)}
        {@const isCurrent = entry === active}
        <div class="tab" class:active={isCurrent}>
          <button
            type="button"
            class="tab-btn"
            aria-pressed={isCurrent}
            title={`${entry.contribution.title} (${entry.plugin})`}
            onclick={() => kernel.openInspectorPlugin(key)}
          >
            {entry.contribution.title}
          </button>
          <button
            type="button"
            class="tab-close"
            aria-label={`Close ${entry.contribution.title}`}
            title={`Close ${entry.contribution.title}`}
            onclick={() => kernel.closeInspectorTab(key, openEntries.map(entryKey))}
          >
            <Icon name="x" size={11} />
          </button>
        </div>
      {:else}
        <span class="heading">Inspector</span>
      {/each}
    </div>

    <div class="menu-wrap" bind:this={menuWrap}>
      <button
        bind:this={triggerBtn}
        type="button"
        class="kk-icon-btn menu-trigger"
        class:active={menuOpen}
        aria-label="Open UI plugin in inspector"
        aria-haspopup="menu"
        aria-expanded={menuOpen}
        title={available.length > 0 ? 'Open or focus a UI plugin panel' : 'No inspector UI plugins available'}
        disabled={available.length === 0}
        onclick={() => (menuOpen = !menuOpen)}
      >
        <Icon name="plus" size={13} />
        <Icon name="chevron-down" size={10} />
      </button>

      {#if menuOpen}
        <div class="menu" role="menu" aria-label="Available inspector panels">
          {#each available as entry (entryKey(entry))}
            {@const key = entryKey(entry)}
            {@const isOpen = openKeySet.has(key)}
            <button
              type="button"
              role="menuitem"
              class="menu-item"
              class:open={isOpen}
              onclick={() => openPlugin(key)}
            >
              <span class="menu-dot" class:on={isOpen} aria-hidden="true"></span>
              <span class="menu-title">{entry.contribution.title}</span>
              <span class="menu-plugin">{entry.plugin}</span>
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </div>

  <div class="body" aria-label={active?.contribution.title ?? 'Inspector'}>
    {#if active}
      {#key scopeKey}
        <SlotHost
          plugin={active.plugin}
          contribution={active.contribution}
          props={panelProps}
          slots={kernel.slots}
          label={active.contribution.title}
        />
      {/key}
    {:else if available.length > 0}
      <div class="kk-empty empty">
        <Icon name="panel" size={22} />
        <p class="msg">Choose a UI plugin from the + menu above.</p>
        <div class="quick">
          {#each available as entry (entryKey(entry))}
            <button type="button" class="kk-button" onclick={() => openPlugin(entryKey(entry))}>
              Open {entry.contribution.title}
            </button>
          {/each}
        </div>
      </div>
    {:else}
      <div class="kk-empty empty">
        <Icon name="panel" size={22} />
        <p class="msg">No inspector panels are active for this workspace.</p>
        <p class="kk-muted sub">Install a UI panel plugin (such as Git Changes) to inspect workspace state alongside your threads.</p>
        <button type="button" class="kk-button" onclick={() => (kernel.main = { kind: 'plugins' })}>Browse plugins</button>
      </div>
    {/if}
  </div>
</div>

<style>
  .inspector {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--kk-surface-frame, Canvas);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 30px;
    padding: 0 6px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    flex: none;
  }
  .tabs {
    display: flex;
    align-items: center;
    gap: 2px;
    flex: 1;
    min-width: 0;
    overflow-x: auto;
  }
  .heading {
    padding: 0 4px;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-secondary, CanvasText);
  }
  .tab {
    display: inline-flex;
    align-items: center;
    border-radius: var(--kk-radius-small, 0);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .tab:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .tab.active {
    background: var(--kk-surface-selected, Highlight);
    color: var(--kk-text-primary, CanvasText);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .tab-btn {
    height: 22px;
    padding: 0 4px 0 8px;
    border: 0;
    background: transparent;
    cursor: pointer;
    white-space: nowrap;
  }
  .tab-close {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 18px;
    height: 18px;
    margin-right: 2px;
    padding: 0;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-muted, GrayText);
    cursor: pointer;
  }
  .tab-close:hover {
    color: var(--kk-text-primary, CanvasText);
  }
  .menu-wrap {
    position: relative;
    flex: none;
  }
  .menu-trigger {
    gap: 2px;
    padding: 0 4px;
  }
  .menu {
    position: absolute;
    top: 100%;
    right: 0;
    margin-top: 4px;
    min-width: 210px;
    max-width: 280px;
    padding: 4px;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    box-shadow: var(--kk-shadow-raised, none);
    z-index: 15;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .menu-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-primary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    text-align: left;
    cursor: pointer;
  }
  .menu-item:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .menu-dot {
    width: 6px;
    height: 6px;
    border-radius: var(--kk-radius-large, 0);
    background: transparent;
    border: 1px solid var(--kk-border-strong, GrayText);
    flex: none;
  }
  .menu-dot.on {
    background: var(--kk-accent, Highlight);
    border-color: var(--kk-accent, Highlight);
  }
  .menu-title {
    font-weight: var(--kk-font-weight-strong, bold);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .menu-plugin {
    margin-left: auto;
    color: var(--kk-text-muted, GrayText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    background: var(--kk-surface-panel, Canvas);
  }
  .empty {
    justify-items: center;
    gap: 10px;
    padding: 20px;
  }
  .quick {
    display: flex;
    flex-wrap: wrap;
    justify-content: center;
    gap: 6px;
  }
  .msg,
  .sub {
    margin: 0;
    max-width: 240px;
    font-size: var(--kk-font-size-small, small);
  }
</style>
