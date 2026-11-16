<script lang="ts">
  let { text }: { text: string } = $props();
</script>

<details class="kk-reasoning">
  <summary>Thinking</summary>
  <p>{text}</p>
</details>

<style>
  .kk-reasoning {
    color: var(--kk-text-secondary, CanvasText);
  }
  p {
    margin: 4px 0 0;
    white-space: pre-wrap;
    font-size: var(--kk-font-size-small, small);
  }
</style>
