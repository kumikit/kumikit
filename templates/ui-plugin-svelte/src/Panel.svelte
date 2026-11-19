<!--
  A sandboxed panel (docs/spec/07-ui-plugins.md#level-2-sandboxed-panels): it reads the
  tab's core through the bridge and follows the theme. It cannot write, reach other cores
  or the network (unless the manifest lists hosts in [ui] network).
-->
<script lang="ts">
  import type { PanelContext } from '@kumikit/ui-sdk/panel';
  import { onMount } from 'svelte';

  let { ctx }: { ctx: PanelContext } = $props();
  let plugins = $state<string[]>([]);
  let error = $state<string | undefined>();

  onMount(() => {
    ctx.connection.call('plugin.list', {}).then(
      (r) => (plugins = (r as { plugins: { name: string }[] }).plugins.map((p) => p.name)),
      (e: unknown) => (error = String(e)),
    );
  });
</script>

<main>
  <h1>Plugins on this core</h1>
  {#if error}<p class="error">{error}</p>{/if}
  <ul>
    {#each plugins as name (name)}<li>{name}</li>{/each}
  </ul>
</main>

<style>
  /* Theme tokens arrive as --kk-* properties and follow the user's theme. */
  main {
    padding: 8px;
    font-family: var(--kk-font-ui, sans-serif);
    color: var(--kk-text-primary, CanvasText);
    background: var(--kk-surface-panel, Canvas);
  }
  h1 {
    font-size: var(--kk-font-size-large, large);
  }
  .error {
    color: var(--kk-state-danger, CanvasText);
  }
</style>
