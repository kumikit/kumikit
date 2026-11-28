<!-- Appearance: mode selector, bundled theme gallery with live SVG preview thumbnails,
     custom `.json` theme file import/removal, and motion preference (08-themes, ADR 0037). -->
<script lang="ts">
  import type { PluginContext, ThemeChoiceInfo, ThemeVariantInfo } from '@kumikit/ui-sdk';

  let { ctx }: { ctx: PluginContext } = $props();
  // Bumped on every theme change, so the reads below are fresh.
  let version = $state(0);
  $effect(() => ctx.theme.onChange(() => version++));

  const choice = $derived.by((): ThemeChoiceInfo => (void version, ctx.theme.choice));
  const variants = $derived.by((): readonly ThemeVariantInfo[] => (void version, ctx.theme.variants));
  const bundled = $derived(variants.filter((v) => v.source !== 'custom'));
  const custom = $derived(variants.filter((v) => v.source === 'custom'));

  let filter = $state<'all' | 'dark' | 'light'>('all');
  const filteredBundled = $derived(
    filter === 'all' ? bundled : bundled.filter((v) => v.appearance === filter),
  );

  const modes = [
    { value: 'mode:system', label: 'Follow system' },
    { value: 'mode:light', label: 'Light' },
    { value: 'mode:dark', label: 'Dark' },
  ];
  const current = $derived(choice.variant ?? `mode:${choice.appearance}`);

  function pick(value: string) {
    if (value.startsWith('mode:')) {
      ctx.theme.select({ variant: null, appearance: value.slice(5) as ThemeChoiceInfo['appearance'] });
    } else {
      ctx.theme.select({ variant: value });
    }
  }

  function palette(v: ThemeVariantInfo) {
    const p = v.preview ?? {};
    const dark = v.appearance === 'dark';
    return {
      frame: p['surface.frame'] ?? (dark ? '#111318' : '#f3f4f6'),
      panel: p['surface.panel'] ?? (dark ? '#181a20' : '#ffffff'),
      card: p['surface.card'] ?? (dark ? '#20232b' : '#f8f9fb'),
      border: p['border.subtle'] ?? (dark ? '#2b2f37' : '#dcdfe6'),
      text: p['text.primary'] ?? (dark ? '#e6e8ee' : '#1b1d23'),
      secondary: p['text.secondary'] ?? (dark ? '#b3b8c4' : '#454a56'),
      muted: p['text.muted'] ?? (dark ? '#8d93a2' : '#676c7a'),
      accent: p['accent'] ?? (dark ? '#7d9df6' : '#3556c4'),
      onAccent: p['text.on-accent'] ?? (dark ? '#0b0d12' : '#ffffff'),
      codeBg: p['code.background'] ?? (dark ? '#101216' : '#f3f4f7'),
      kw: p['syntax.keyword'] ?? (dark ? '#c792ea' : '#8a2fb5'),
      str: p['syntax.string'] ?? (dark ? '#a5d68a' : '#1b7442'),
      fn: p['syntax.function'] ?? (dark ? '#82aaff' : '#2650c2'),
      num: p['syntax.number'] ?? (dark ? '#f5a76c' : '#a6530a'),
    };
  }
</script>

{#snippet thumbnail(v: ThemeVariantInfo)}
  {@const c = palette(v)}
  <svg class="thumb" viewBox="0 0 144 86" fill="none" aria-hidden="true">
    <!-- Frame and main panel -->
    <rect x="0.5" y="0.5" width="143" height="85" rx="5" fill={c.frame} stroke={c.border} />
    <rect x="34" y="1" width="109" height="84" rx="4" fill={c.panel} />
    <line x1="34" y1="1" x2="34" y2="85" stroke={c.border} />
    <!-- Sidebar items -->
    <circle cx="9" cy="9" r="2.2" fill={c.muted} />
    <circle cx="16" cy="9" r="2.2" fill={c.muted} />
    <circle cx="23" cy="9" r="2.2" fill={c.muted} />
    <rect x="6" y="19" width="22" height="5" rx="2" fill={c.accent} />
    <rect x="6" y="29" width="18" height="3.5" rx="1.5" fill={c.secondary} />
    <rect x="6" y="37" width="21" height="3.5" rx="1.5" fill={c.muted} />
    <rect x="6" y="45" width="15" height="3.5" rx="1.5" fill={c.muted} />
    <!-- Top bar inside panel -->
    <rect x="42" y="8" width="38" height="4" rx="2" fill={c.text} />
    <rect x="116" y="7" width="20" height="7" rx="3" fill={c.accent} />
    <rect x="120" y="9.5" width="12" height="2" rx="1" fill={c.onAccent} />
    <line x1="34" y1="19" x2="143" y2="19" stroke={c.border} />
    <!-- Conversation card & text lines -->
    <rect x="42" y="25" width="93" height="22" rx="4" fill={c.card} stroke={c.border} />
    <rect x="48" y="30" width="52" height="3.5" rx="1.5" fill={c.text} />
    <rect x="48" y="37" width="74" height="3" rx="1.5" fill={c.secondary} />
    <!-- Code snippet preview with syntax accents -->
    <rect x="42" y="52" width="93" height="26" rx="4" fill={c.codeBg} stroke={c.border} />
    <rect x="48" y="58" width="16" height="3" rx="1.5" fill={c.kw} />
    <rect x="68" y="58" width="24" height="3" rx="1.5" fill={c.fn} />
    <rect x="96" y="58" width="14" height="3" rx="1.5" fill={c.num} />
    <rect x="48" y="65" width="34" height="3" rx="1.5" fill={c.str} />
    <rect x="86" y="65" width="22" height="3" rx="1.5" fill={c.muted} />
    <rect x="48" y="71" width="18" height="2.5" rx="1" fill={c.accent} />
    <rect x="70" y="71" width="30" height="2.5" rx="1" fill={c.text} />
  </svg>
{/snippet}

<div class="kk-appearance">
  <div class="header">
    <div class="intro">
      <h2>Appearance</h2>
      <p class="muted">Choose a bundled color theme or import a <code>.json</code> theme file from disk.</p>
    </div>
    {#if ctx.theme.importFromFile}
      <button type="button" class="add-btn" onclick={() => void ctx.theme.importFromFile?.()}>
        Add theme…
      </button>
    {/if}
  </div>

  <fieldset class="section">
    <legend>Default mode</legend>
    <div class="modes">
      {#each modes as m (m.value)}
        <label class="mode" class:selected={current === m.value}>
          <input type="radio" name="kk-theme" value={m.value} checked={current === m.value} onchange={() => pick(m.value)} />
          <span>{m.label}</span>
        </label>
      {/each}
    </div>
  </fieldset>

  {#if custom.length > 0}
    <fieldset class="section">
      <legend>Your themes ({custom.length})</legend>
      <div class="grid">
        {#each custom as v (v.key)}
          <div class="card-wrap" class:selected={current === v.key}>
            <label class="theme-card">
              <input
                type="radio"
                class="sr-only"
                name="kk-theme"
                value={v.key}
                checked={current === v.key}
                onchange={() => pick(v.key)}
              />
              {@render thumbnail(v)}
              <span class="card-meta">
                <span class="card-name">{v.name}</span>
                <span class="chip">{v.appearance}</span>
              </span>
            </label>
            {#if ctx.theme.remove}
              <button
                type="button"
                class="remove-btn"
                aria-label={`Remove ${v.name}`}
                onclick={() => ctx.theme.remove?.(v.key)}
              >
                Remove
              </button>
            {/if}
          </div>
        {/each}
      </div>
    </fieldset>
  {/if}

  <fieldset class="section">
    <div class="section-bar">
      <legend>Themes ({filteredBundled.length})</legend>
      <div class="filters" role="group" aria-label="Filter themes by appearance">
        {#each [['all', 'All'], ['dark', 'Dark'], ['light', 'Light']] as const as [val, label] (val)}
          <button
            type="button"
            class="filter-btn"
            class:active={filter === val}
            aria-pressed={filter === val}
            onclick={() => (filter = val)}
          >
            {label}
          </button>
        {/each}
      </div>
    </div>
    <div class="grid">
      {#each filteredBundled as v (v.key)}
        <div class="card-wrap" class:selected={current === v.key}>
          <label class="theme-card">
            <input
              type="radio"
              class="sr-only"
              name="kk-theme"
              value={v.key}
              checked={current === v.key}
              onchange={() => pick(v.key)}
            />
            {@render thumbnail(v)}
            <span class="card-meta">
              <span class="card-name">{v.name}</span>
              <span class="chip">{v.appearance}</span>
            </span>
          </label>
        </div>
      {/each}
    </div>
  </fieldset>

  <label class="option check">
    <input type="checkbox" checked={choice.reduceMotion} onchange={(e) => ctx.theme.select({ reduceMotion: e.currentTarget.checked })} />
    <span>Reduce motion, even if the system does not ask for it</span>
  </label>
</div>

<style>
  .kk-appearance {
    display: grid;
    gap: 20px;
    max-width: 780px;
  }
  .header {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
    flex-wrap: wrap;
  }
  .intro {
    display: grid;
    gap: 4px;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
  }
  .intro p {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
  }
  .add-btn {
    padding: 6px 12px;
    border: 1px solid var(--kk-border-strong, ButtonBorder);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, ButtonFace);
    color: var(--kk-text-primary, ButtonText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    cursor: pointer;
    flex: none;
  }
  .add-btn:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .add-btn:focus-visible,
  .filter-btn:focus-visible,
  .remove-btn:focus-visible {
    outline: 2px solid var(--kk-focus, Highlight);
    outline-offset: 2px;
  }
  fieldset.section {
    border: 0;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 10px;
  }
  .section-bar {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 10px;
  }
  legend {
    padding: 0;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-secondary, CanvasText);
  }
  .modes {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .mode {
    display: inline-flex;
    align-items: center;
    gap: 8px;
    padding: 7px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    cursor: pointer;
    font-size: var(--kk-font-size-small, small);
  }
  .mode:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .mode.selected {
    border-color: var(--kk-accent, Highlight);
    background: var(--kk-surface-selected, Highlight);
  }
  .filters {
    display: inline-flex;
    gap: 4px;
  }
  .filter-btn {
    padding: 3px 9px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, ButtonFace);
    color: var(--kk-text-secondary, ButtonText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .filter-btn:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, ButtonText);
  }
  .filter-btn.active {
    border-color: var(--kk-accent, Highlight);
    background: var(--kk-surface-selected, Highlight);
    color: var(--kk-text-primary, ButtonText);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(168px, 1fr));
    gap: 10px;
  }
  .card-wrap {
    display: flex;
    flex-direction: column;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    overflow: hidden;
  }
  .card-wrap:hover {
    border-color: var(--kk-border-strong, CanvasText);
    background: var(--kk-surface-hover, ButtonFace);
  }
  .card-wrap.selected {
    border-color: var(--kk-accent, Highlight);
    background: var(--kk-surface-selected, Highlight);
    box-shadow: 0 0 0 1px var(--kk-accent, Highlight);
  }
  .card-wrap:has(input:focus-visible) {
    outline: 2px solid var(--kk-focus, Highlight);
    outline-offset: 2px;
  }
  .theme-card {
    display: grid;
    gap: 8px;
    padding: 8px;
    cursor: pointer;
  }
  .thumb {
    display: block;
    width: 100%;
    height: auto;
    border-radius: var(--kk-radius-small, 0);
  }
  .card-meta {
    display: flex;
    align-items: center;
    gap: 6px;
    min-width: 0;
    font-size: var(--kk-font-size-small, small);
  }
  .card-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
  }
  .chip {
    margin-left: auto;
    padding: 1px 6px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-muted, GrayText);
    flex: none;
  }
  .remove-btn {
    align-self: flex-end;
    margin: 0 8px 8px;
    padding: 2px 8px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-panel, ButtonFace);
    color: var(--kk-state-danger, ButtonText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .remove-btn:hover {
    background: var(--kk-state-danger-tint, ButtonFace);
  }
  .option {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 9px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    cursor: pointer;
    font-size: var(--kk-font-size-small, small);
  }
  .option:hover {
    background: var(--kk-surface-hover, ButtonFace);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    padding: 0;
    margin: -1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
    border: 0;
  }
  .muted {
    color: var(--kk-text-muted, GrayText);
  }
</style>
