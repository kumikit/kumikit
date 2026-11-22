<!-- Appearance: follow the system, or pick light, dark or a theme variant. -->
<script lang="ts">
  import type { PluginContext, ThemeChoiceInfo } from '@kumikit/ui-sdk';

  let { ctx }: { ctx: PluginContext } = $props();
  // Bumped on every theme change, so the reads below are fresh.
  let version = $state(0);
  $effect(() => ctx.theme.onChange(() => version++));
  const choice = $derived.by((): ThemeChoiceInfo => (void version, ctx.theme.choice));
  const variants = $derived.by(() => (void version, ctx.theme.variants));
  const modes = [
    { value: 'mode:system', label: 'Follow the system' },
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
</script>

<div class="kk-appearance">
  <div class="intro">
    <h2>Appearance</h2>
    <p class="muted">Choose a color theme and motion preference for this window.</p>
  </div>
  <fieldset>
    <legend>Theme</legend>
    <div class="options">
      {#each modes as m (m.value)}
        <label class="option" class:selected={current === m.value}>
          <input type="radio" name="kk-theme" value={m.value} checked={current === m.value} onchange={() => pick(m.value)} />
          <span>{m.label}</span>
        </label>
      {/each}
      {#each variants as v (v.key)}
        <label class="option" class:selected={current === v.key}>
          <input type="radio" name="kk-theme" value={v.key} checked={current === v.key} onchange={() => pick(v.key)} />
          <span>{v.name}</span>
          <span class="chip">{v.appearance}</span>
        </label>
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
    gap: 18px;
    max-width: 580px;
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
  fieldset {
    border: 0;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 8px;
  }
  legend {
    padding: 0;
    margin-bottom: 4px;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-secondary, CanvasText);
  }
  .options {
    display: grid;
    gap: 6px;
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
  .option.selected {
    border-color: var(--kk-accent, Highlight);
    background: var(--kk-surface-selected, Highlight);
  }
  .chip {
    margin-left: auto;
    padding: 1px 7px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-muted, GrayText);
  }
  .muted {
    color: var(--kk-text-muted, GrayText);
  }
</style>
