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
    <span class="dot {state}" aria-hidden="true"></span>
    <span class="name">{item.name}</span>
    {#if summary}<code>{summary}</code>{/if}
    <span class="state">{state === 'running' ? 'running…' : state === 'failed' ? 'failed' : 'done'}</span>
  </summary>
  <pre>{JSON.stringify(item.input, null, 2)}</pre>
  {#if item.output}<pre class="out">{item.output}</pre>{/if}
  {#if item.result !== undefined}<pre class="out">{JSON.stringify(item.result, null, 2)}</pre>{/if}
</details>

<style>
  .kk-tool {
    padding: 5px 10px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    font-size: var(--kk-font-size-small, small);
  }
  summary {
    display: flex;
    gap: 7px;
    align-items: center;
    cursor: pointer;
    list-style: none;
  }
  .dot {
    width: 6px;
    height: 6px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-state-success, Highlight);
    flex: none;
  }
  .dot.running {
    background: var(--kk-state-info, Highlight);
  }
  .dot.failed {
    background: var(--kk-state-danger, CanvasText);
  }
  .name {
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
    white-space: nowrap;
    flex: none;
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
    white-space: nowrap;
  }
  .failed .state {
    color: var(--kk-state-danger, CanvasText);
  }
  pre {
    overflow: auto;
    max-height: 300px;
    margin: 8px 0 0;
    padding: 8px 10px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-code-background, Canvas);
  }
</style>
