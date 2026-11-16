<!-- Manage plugins (06-client, Plugin manager). Filled in with installing and granting. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  let { kernel }: { kernel: Kernel } = $props();
</script>

<div class="plugins">
  <h1>Plugins</h1>
  <ul>
    {#each kernel.connection.plugins as p (p.id)}
      <li>{p.id} <span class="kk-muted">{p.version}</span></li>
    {/each}
  </ul>
</div>

<style>
  .plugins {
    padding: 16px 24px;
  }
  h1 {
    font-size: var(--kk-font-size-title, x-large);
  }
</style>
