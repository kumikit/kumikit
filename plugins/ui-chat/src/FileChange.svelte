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
  }
  figcaption {
    margin-bottom: 4px;
  }
  pre {
    overflow: auto;
    max-height: 360px;
    margin: 0;
    padding: 6px 8px;
    border-radius: var(--kk-radius-small, 0);
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
