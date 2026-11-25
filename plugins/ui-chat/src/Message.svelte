<script lang="ts">
  import type { PluginContext } from '@kumikit/ui-sdk';
  let { ctx, text, done }: { ctx: PluginContext; text: string; done: boolean } = $props();

  function attachCopyButtons(el: HTMLElement) {
    for (const pre of Array.from(el.querySelectorAll('pre'))) {
      if (pre.querySelector('.kk-copy-code')) continue;
      const btn = el.ownerDocument.createElement('button');
      btn.type = 'button';
      btn.className = 'kk-copy-code';
      btn.textContent = 'Copy';
      btn.setAttribute('aria-label', 'Copy code');
      btn.onclick = () => {
        const code = pre.querySelector('code')?.textContent ?? pre.textContent ?? '';
        if (typeof navigator !== 'undefined' && navigator.clipboard) {
          void navigator.clipboard.writeText(code).then(
            () => {
              btn.textContent = 'Copied';
              setTimeout(() => {
                btn.textContent = 'Copy';
              }, 1200);
            },
            () => {},
          );
        }
      };
      pre.append(btn);
    }
  }

  function md(el: HTMLElement, t: string) {
    ctx.ui.markdown(el, t);
    attachCopyButtons(el);
    return {
      update: (next: string) => {
        ctx.ui.markdown(el, next);
        attachCopyButtons(el);
      },
    };
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
    position: relative;
    overflow: auto;
    margin: 8px 0;
    padding: 10px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-code-background, Canvas);
    color: var(--kk-code-text, CanvasText);
  }
  .kk-msg :global(.kk-copy-code) {
    position: absolute;
    top: 6px;
    right: 6px;
    padding: 2px 6px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-raised, ButtonFace);
    color: var(--kk-text-secondary, ButtonText);
    font-family: var(--kk-font-sans, sans-serif);
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .kk-msg :global(.kk-copy-code:hover) {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, ButtonText);
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
