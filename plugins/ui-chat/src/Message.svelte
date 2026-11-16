<script lang="ts">
  import type { PluginContext } from '@kumikit/ui-sdk';
  let { ctx, text, done }: { ctx: PluginContext; text: string; done: boolean } = $props();
  function md(el: HTMLElement, t: string) {
    ctx.ui.markdown(el, t);
    return { update: (next: string) => ctx.ui.markdown(el, next) };
  }
</script>

<div class="kk-msg" aria-busy={!done} use:md={text}></div>

<style>
  .kk-msg :global(p) {
    margin: 0 0 8px;
  }
  .kk-msg :global(pre) {
    overflow: auto;
    padding: 8px 10px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-code-background, Canvas);
    color: var(--kk-code-text, CanvasText);
  }
  .kk-msg :global(code) {
    font-family: var(--kk-font-mono, monospace);
  }
  .kk-msg :global(a) {
    color: var(--kk-text-link, LinkText);
  }
</style>
