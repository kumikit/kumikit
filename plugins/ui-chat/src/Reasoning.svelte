<script lang="ts">
  let { text }: { text: string } = $props();
</script>

<details class="kk-reasoning">
  <summary>Thinking</summary>
  <p>{text}</p>
</details>

<style>
  .kk-reasoning {
    padding: 4px 10px;
    border-left: 2px solid var(--kk-border-subtle, CanvasText);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  summary {
    cursor: pointer;
    color: var(--kk-text-muted, GrayText);
    user-select: none;
  }
  summary:hover {
    color: var(--kk-text-secondary, CanvasText);
  }
  p {
    margin: 6px 0 2px;
    white-space: pre-wrap;
  }
</style>
