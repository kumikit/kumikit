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
  <h2>Appearance</h2>
  <fieldset>
    <legend>Theme</legend>
    {#each modes as m (m.value)}
      <label><input type="radio" name="kk-theme" value={m.value} checked={current === m.value} onchange={() => pick(m.value)} /> {m.label}</label>
    {/each}
    {#each variants as v (v.key)}
      <label><input type="radio" name="kk-theme" value={v.key} checked={current === v.key} onchange={() => pick(v.key)} /> {v.name} <span class="muted">({v.appearance})</span></label>
    {/each}
  </fieldset>
  <label class="check">
    <input type="checkbox" checked={choice.reduceMotion} onchange={(e) => ctx.theme.select({ reduceMotion: e.currentTarget.checked })} />
    Reduce motion, even if the system does not ask for it
  </label>
</div>

<style>
  .kk-appearance {
    display: grid;
    gap: 16px;
    max-width: 560px;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
  }
  fieldset {
    display: grid;
    gap: 6px;
    margin: 0;
    padding: 8px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
  }
  label {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .muted {
    color: var(--kk-text-muted, GrayText);
  }
</style>
