<!-- The model picker (06-client, Model picker; ADR 0033): a popover anchored to a composer's
     model button with search across every provider's models, recent models, the core's
     providers with their icons, the selected provider's models, and the chosen model's
     settings (effort, speed, …) as rows of segments. Picking a model commits and closes;
     changing a setting keeps it open; Escape or a click outside commits what changed. -->
<script lang="ts">
  import type { ModelInfo, ProviderInfo } from '@kumikit/protocol';
  import { isComposing, type ModelPick } from '@kumikit/ui-sdk';
  import { onMount, untrack } from 'svelte';
  import {
    keepValidOptions,
    modelLabel,
    optionChoices,
    orderOptions,
    searchModels,
    type ModelEntry,
    type RecentModel,
  } from './model-picker';
  import ProviderIcon from './ProviderIcon.svelte';

  let {
    providers,
    loadModels,
    initial,
    recents,
    anchor,
    onDone,
  }: {
    providers: readonly ProviderInfo[];
    loadModels: (provider: string) => Promise<ModelInfo[]>;
    initial: { provider?: string | undefined; model?: string | undefined; options: Readonly<Record<string, string>> };
    recents: readonly RecentModel[];
    anchor: HTMLElement;
    onDone: (pick: ModelPick | undefined) => void;
  } = $props();

  const RECENT = '\u0000recent';
  let root: HTMLElement | undefined = $state();
  let search: HTMLInputElement | undefined = $state();
  let query = $state('');
  let highlight = $state(0);
  let models = $state<Record<string, ModelInfo[] | 'loading' | 'error'>>({});
  // svelte-ignore state_referenced_locally
  let provider = $state(initial.provider ?? providers[0]?.id ?? '');
  // svelte-ignore state_referenced_locally
  let model = $state(initial.model ?? '');
  // svelte-ignore state_referenced_locally
  let options = $state<Record<string, string>>({ ...initial.options });
  // svelte-ignore state_referenced_locally
  let view = $state(initial.provider ? initial.provider : recents.length > 0 ? RECENT : (providers[0]?.id ?? ''));
  let done = false;
  const pickerId = `kk-model-picker-${Math.random().toString(36).slice(2, 8)}`;

  function ensure(id: string) {
    if (!id || models[id]) return;
    models[id] = 'loading';
    loadModels(id).then(
      (list) => {
        models[id] = list;
      },
      () => {
        models[id] = 'error';
      },
    );
  }

  const listOf = (id: string): ModelInfo[] | undefined => {
    const m = models[id];
    return Array.isArray(m) ? m : undefined;
  };
  const providerOf = (id: string) => providers.find((p) => p.id === id);

  $effect(() => {
    ensure(provider);
    if (view !== RECENT) ensure(view);
    else for (const r of recents) ensure(r.provider);
  });
  // Search looks through every provider's models.
  $effect(() => {
    if (query.trim()) for (const p of providers) untrack(() => ensure(p.id));
  });

  const current = $derived(listOf(provider)?.find((m) => m.id === model));
  const settings = $derived(orderOptions(current?.options));

  const entries = $derived.by<ModelEntry[]>(() => {
    if (query.trim()) {
      const all: ModelEntry[] = [];
      for (const p of providers) for (const m of listOf(p.id) ?? []) all.push({ provider: p, model: m });
      return searchModels(all, query);
    }
    if (view === RECENT) {
      return recents.flatMap((r) => {
        const p = providerOf(r.provider);
        if (!p) return [];
        const m = listOf(r.provider)?.find((x) => x.id === r.model) ?? { id: r.model, label: r.model, capabilities: [] };
        return [{ provider: p, model: m }];
      });
    }
    const p = providerOf(view);
    return p ? (listOf(view) ?? []).map((m) => ({ provider: p, model: m })) : [];
  });
  const loading = $derived(
    query.trim() ? providers.some((p) => models[p.id] === 'loading') : view !== RECENT && models[view] === 'loading',
  );
  const failed = $derived(!query.trim() && view !== RECENT && models[view] === 'error');
  const total = $derived(providers.reduce((n, p) => n + (listOf(p.id)?.length ?? 0), 0));
  const placeholder = $derived(
    total > 0 && providers.every((p) => listOf(p.id))
      ? `Search ${total} ${total === 1 ? 'model' : 'models'}, ${providers.length} ${providers.length === 1 ? 'agent' : 'agents'}…`
      : `Search models in ${providers.length} ${providers.length === 1 ? 'agent' : 'agents'}…`,
  );
  $effect(() => {
    void entries.length;
    if (highlight >= entries.length) highlight = Math.max(0, entries.length - 1);
  });

  function finish() {
    if (done) return;
    done = true;
    const changed =
      provider !== initial.provider ||
      model !== initial.model ||
      JSON.stringify(Object.entries(options).sort()) !== JSON.stringify(Object.entries(initial.options).sort());
    onDone(changed && provider && model ? { provider, model, options: { ...options } } : undefined);
  }

  function choose(e: ModelEntry) {
    provider = e.provider.id;
    model = e.model.id;
    options = keepValidOptions(e.model, options);
    finish();
  }

  function setOption(id: string, value: string | undefined) {
    const next = { ...options };
    if (value === undefined) delete next[id];
    else next[id] = value;
    options = next;
  }

  function showView(id: string) {
    view = id;
    query = '';
    highlight = 0;
  }

  function onKey(e: KeyboardEvent) {
    if (isComposing(e)) return;
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      finish();
    } else if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      if (entries.length === 0) return;
      e.preventDefault();
      highlight = (highlight + (e.key === 'ArrowDown' ? 1 : entries.length - 1)) % entries.length;
      root?.querySelector<HTMLElement>(`[data-entry="${highlight}"]`)?.scrollIntoView({ block: 'nearest' });
    } else if (e.key === 'Enter' && e.target === search) {
      const entry = entries[highlight];
      if (entry) {
        e.preventDefault();
        choose(entry);
      }
    }
  }

  function onWindowPointerDown(e: PointerEvent) {
    const t = e.target as Node;
    if (root && !root.contains(t) && !anchor.contains(t)) finish();
  }

  // Position above the anchor (or below when there is no room), through a constructed
  // stylesheet so CSP `style-src 'self'` stays strict.
  function place() {
    if (typeof document === 'undefined' || !('adoptedStyleSheets' in document) || !root) return () => {};
    const r = anchor.getBoundingClientRect();
    const w = root.offsetWidth || 540;
    const h = root.offsetHeight || 480;
    const left = Math.round(Math.max(8, Math.min(r.left, window.innerWidth - w - 8)));
    const above = r.top - 8 >= h || r.top > window.innerHeight - r.bottom;
    const rule = above
      ? `bottom: ${Math.round(window.innerHeight - r.top + 6)}px; max-height: ${Math.round(r.top - 14)}px;`
      : `top: ${Math.round(r.bottom + 6)}px; max-height: ${Math.round(window.innerHeight - r.bottom - 14)}px;`;
    const sheet = new CSSStyleSheet();
    sheet.replaceSync(`[data-model-picker="${pickerId}"] { left: ${left}px; ${rule} }`);
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
    return () => {
      document.adoptedStyleSheets = document.adoptedStyleSheets.filter((s) => s !== sheet);
    };
  }

  onMount(() => {
    const unplace = place();
    search?.focus();
    return unplace;
  });

  const hint = $derived.by(() => {
    for (const o of settings) {
      const c = optionChoices(o).find((x) => x.value === options[o.id]);
      if (c?.description) return c.description;
    }
    return settings.some((o) => options[o.id] === undefined) ? "Unset settings use the agent's own default." : '';
  });
</script>

<svelte:window onpointerdown={onWindowPointerDown} onresize={finish} />

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  bind:this={root}
  class="kk-model-picker"
  data-model-picker={pickerId}
  role="dialog"
  aria-label="Choose a model"
  tabindex="-1"
  onkeydown={onKey}
>
  <div class="search">
    <svg width="14" height="14" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" aria-hidden="true">
      <circle cx="7" cy="7" r="4.5" />
      <path d="M10.5 10.5 14 14" />
    </svg>
    <input
      bind:this={search}
      bind:value={query}
      type="search"
      aria-label="Search models"
      aria-controls={`${pickerId}-models`}
      {placeholder}
      oninput={() => (highlight = 0)}
    />
  </div>
  <div class="columns">
    <nav class="agents" aria-label="Agents">
      {#if recents.length > 0}
        <button type="button" class="agent" class:selected={!query && view === RECENT} aria-current={!query && view === RECENT ? 'true' : undefined} onclick={() => showView(RECENT)}>
          <svg class="recent-ico" width="16" height="16" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
            <path d="M2.5 8a5.5 5.5 0 1 0 1.6-3.9" />
            <path d="M2.5 2.5v2.6h2.6" />
            <path d="M8 5.2V8l2 1.3" />
          </svg>
          <span class="agent-name">Recent</span>
        </button>
      {/if}
      <p class="heading">Agents</p>
      {#each providers as p (p.id)}
        <button
          type="button"
          class="agent"
          class:selected={!query && view === p.id}
          aria-current={!query && view === p.id ? 'true' : undefined}
          onclick={() => showView(p.id)}
        >
          <ProviderIcon name={p.name} icon={p.icon} size="medium" />
          <span class="agent-name">{p.name}</span>
          {#if p.id === provider}<span class="dot" aria-hidden="true"></span><span class="kk-sr-only">(current)</span>{/if}
        </button>
      {/each}
    </nav>
    <section class="models" aria-label="Models" id={`${pickerId}-models`}>
      <p class="heading models-heading">
        {#if query.trim()}
          Results
        {:else if view === RECENT}
          Recent
        {:else}
          {@const vp = providerOf(view)}
          {#if vp}<ProviderIcon name={vp.name} icon={vp.icon} />{vp.name}{/if}
        {/if}
      </p>
      <div class="model-list">
        {#each entries as e, i (`${e.provider.id}/${e.model.id}`)}
          {@const isCurrent = e.provider.id === provider && e.model.id === model}
          <button
            type="button"
            class="model"
            class:highlight={i === highlight}
            data-entry={i}
            aria-pressed={isCurrent}
            title={e.model.id}
            onpointermove={() => (highlight = i)}
            onclick={() => choose(e)}
          >
            {#if query.trim() || view === RECENT}<ProviderIcon name={e.provider.name} icon={e.provider.icon} />{/if}
            <span class="model-label">{e.model.label}</span>
            {#if query.trim() || view === RECENT}<span class="model-agent">{e.provider.name}</span>{/if}
            {#if isCurrent}
              <svg class="check" width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                <path d="M3 8.5 6.5 12 13 4.5" />
              </svg>
            {/if}
          </button>
        {:else}
          <p class="empty">
            {#if loading}Loading models…{:else if failed}This agent could not list its models.{:else if query.trim()}No model matches.{:else}No models.{/if}
          </p>
        {/each}
      </div>
    </section>
  </div>
  {#if settings.length > 0}
    <div class="settings" aria-label={`Settings for ${modelLabel(listOf(provider), model)}`} role="group">
      {#each settings as o (o.id)}
        <div class="setting" role="radiogroup" aria-label={o.label}>
          <span class="setting-label">{o.label}</span>
          <div class="segments">
            {#each optionChoices(o) as c (c.value ?? '')}
              <button
                type="button"
                role="radio"
                class="segment"
                aria-checked={options[o.id] === c.value}
                title={c.description ?? c.label}
                onclick={() => setOption(o.id, c.value)}
              >
                {c.label}
              </button>
            {/each}
          </div>
        </div>
      {/each}
      {#if hint}<p class="hint">{hint}</p>{/if}
    </div>
  {/if}
</div>

<style>
  .kk-model-picker {
    position: fixed;
    z-index: 60;
    width: min(540px, calc(100vw - 16px));
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 8px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-primary, CanvasText);
    box-shadow: var(--kk-shadow-overlay, none);
    font-size: var(--kk-font-size-base, medium);
    overflow: hidden;
  }
  .kk-model-picker:focus {
    outline: none;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-input, Field);
    color: var(--kk-text-muted, GrayText);
  }
  .search:focus-within {
    border-color: var(--kk-focus, Highlight);
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    color: var(--kk-text-primary, FieldText);
    font: inherit;
  }
  .columns {
    flex: 1;
    min-height: 0;
    display: grid;
    grid-template-columns: minmax(150px, 38%) 1fr;
    gap: 8px;
  }
  .agents,
  .models {
    min-height: 0;
    max-height: 340px;
    overflow: auto;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .models {
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    padding: 4px;
  }
  .model-list {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .heading {
    margin: 6px 8px 2px;
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .models-heading {
    margin-top: 2px;
  }
  .agent,
  .model {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 6px 8px;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-medium, 0);
    background: transparent;
    color: var(--kk-text-primary, ButtonText);
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .agent:hover,
  .model.highlight {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .agent.selected {
    background: var(--kk-surface-selected, Highlight);
  }
  .model[aria-pressed='true'] {
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .recent-ico {
    flex: none;
    color: var(--kk-text-secondary, CanvasText);
  }
  .agent-name,
  .model-label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .dot {
    flex: none;
    width: 6px;
    height: 6px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-text-secondary, CanvasText);
  }
  .model-agent {
    flex: none;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .check {
    flex: none;
    color: var(--kk-text-secondary, CanvasText);
  }
  .empty {
    margin: 10px 8px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .settings {
    display: flex;
    flex-direction: column;
    gap: 6px;
    padding: 6px 4px 2px;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .setting {
    display: grid;
    grid-template-columns: 64px 1fr;
    align-items: center;
    gap: 8px;
  }
  .setting-label {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .segments {
    display: flex;
    gap: 2px;
    padding: 2px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  .segment {
    flex: 1;
    min-width: 0;
    padding: 4px 6px;
    border: 1px solid transparent;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-secondary, ButtonText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
    cursor: pointer;
  }
  .segment:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .segment[aria-checked='true'] {
    border-color: var(--kk-border-subtle, ButtonBorder);
    background: var(--kk-surface-selected, Highlight);
    color: var(--kk-text-primary, ButtonText);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .hint {
    margin: 2px 0 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
</style>
