<script lang="ts">
  import type { TranscriptItem } from '@kumikit/ui-sdk';
  let { item }: { item: Extract<TranscriptItem, { kind: 'tool' }> } = $props();
  const state = $derived(item.ok === undefined ? 'running' : item.ok ? 'done' : 'failed');
  const summary = $derived.by(() => {
    const input = item.input;
    if (input && typeof input === 'object') {
      const o = input as Record<string, unknown>;
      for (const k of ['command', 'cmd', 'path', 'file_path', 'pattern', 'url']) if (typeof o[k] === 'string') return o[k];
    }
    return '';
  });
</script>

<details class="kk-tool {state}">
  <summary>
    <span class="name">{item.name}</span>
    {#if summary}<code>{summary}</code>{/if}
    <span class="state">{state === 'running' ? 'running…' : state === 'failed' ? 'failed' : ''}</span>
  </summary>
  <pre>{JSON.stringify(item.input, null, 2)}</pre>
  {#if item.output}<pre class="out">{item.output}</pre>{/if}
  {#if item.result !== undefined}<pre class="out">{JSON.stringify(item.result, null, 2)}</pre>{/if}
</details>

<style>
  .kk-tool {
    padding: 6px 10px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  summary {
    display: flex;
    gap: 8px;
    align-items: baseline;
    cursor: pointer;
  }
  .name {
    font-weight: var(--kk-font-weight-strong, bold);
  }
  code {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--kk-text-secondary, CanvasText);
  }
  .state {
    margin-left: auto;
    color: var(--kk-text-muted, GrayText);
  }
  .failed .state {
    color: var(--kk-state-danger, CanvasText);
  }
  pre {
    overflow: auto;
    max-height: 320px;
    margin: 6px 0 0;
    padding: 6px 8px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-code-background, Canvas);
  }
</style>
