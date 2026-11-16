<script lang="ts">
  let { plugin, event, data }: { plugin: string; event: string; data: unknown } = $props();
</script>

<details class="kk-plugin-event">
  <summary><code>{event}</code> <span>from {plugin}</span></summary>
  <pre>{JSON.stringify(data, null, 2)}</pre>
</details>

<style>
  .kk-plugin-event {
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  pre {
    overflow: auto;
    margin: 4px 0 0;
  }
</style>
