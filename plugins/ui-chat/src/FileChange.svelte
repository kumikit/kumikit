<script lang="ts">
  let { path, diff }: { path: string; diff?: string } = $props();
  const lines = $derived((diff ?? '').split('\n'));
  const cls = (l: string) => (l.startsWith('+') && !l.startsWith('+++') ? 'add' : l.startsWith('-') && !l.startsWith('---') ? 'del' : '');
</script>

<figure class="kk-file">
  <figcaption>Changed <code>{path}</code></figcaption>
  {#if diff}
    <pre aria-label={`Changes to ${path}`}>{#each lines as l, i (i)}<span class={cls(l)}>{l}
</span>{/each}</pre>
  {/if}
</figure>

<style>
  .kk-file {
    margin: 0;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    overflow: hidden;
  }
  figcaption {
    padding: 5px 10px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
  }
  pre {
    overflow: auto;
    max-height: 360px;
    margin: 0;
    padding: 8px 10px;
    background: var(--kk-code-background, Canvas);
  }
  .add {
    color: var(--kk-diff-added, CanvasText);
    background: var(--kk-diff-added-tint, Canvas);
  }
  .del {
    color: var(--kk-diff-removed, CanvasText);
    background: var(--kk-diff-removed-tint, Canvas);
  }
</style>
