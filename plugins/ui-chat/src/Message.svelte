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
  .kk-msg {
    color: var(--kk-text-primary, CanvasText);
    line-height: var(--kk-line-height-base, normal);
    overflow-wrap: anywhere;
  }
  .kk-msg :global(p) {
    margin: 0 0 10px;
  }
  .kk-msg :global(p:last-child) {
    margin-bottom: 0;
  }
  .kk-msg :global(h1),
  .kk-msg :global(h2),
  .kk-msg :global(h3) {
    margin: 14px 0 6px;
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .kk-msg :global(ul),
  .kk-msg :global(ol) {
    margin: 6px 0 10px;
    padding-left: 20px;
  }
  .kk-msg :global(li) {
    margin: 3px 0;
  }
  .kk-msg :global(blockquote) {
    margin: 8px 0;
    padding: 4px 12px;
    border-left: 3px solid var(--kk-border-strong, CanvasText);
    color: var(--kk-text-secondary, CanvasText);
  }
  .kk-msg :global(pre) {
    overflow: auto;
    margin: 8px 0;
    padding: 10px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-code-background, Canvas);
    color: var(--kk-code-text, CanvasText);
  }
  .kk-msg :global(code) {
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
  }
  .kk-msg :global(:not(pre) > code) {
    padding: 1px 4px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-code-background, Canvas);
  }
  .kk-msg :global(a) {
    color: var(--kk-text-link, LinkText);
  }
</style>
