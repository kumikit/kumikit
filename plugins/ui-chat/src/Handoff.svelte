<!-- Bounded conversation context carried forward from an earlier provider (ADR 0029),
     expandable to the exact text sent to the new agent. -->
<script lang="ts">
  import type { TranscriptItem } from '@kumikit/ui-sdk';

  type HandoffItem = Extract<TranscriptItem, { kind: 'handoff' }>;

  let { item }: { item: HandoffItem } = $props();

  const label = $derived(
    `Conversation carried forward from ${item.from.provider} (${item.from.model}) · ${item.messages} ${item.messages === 1 ? 'message' : 'messages'}${item.truncated ? ' (trimmed)' : ''}`,
  );
</script>

<details class="kk-handoff" aria-label="Conversation handoff">
  <summary>{label}</summary>
  <pre class="rendered">{item.rendered}</pre>
</details>

<style>
  .kk-handoff {
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-raised, Canvas);
    padding: 6px 10px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  summary {
    cursor: pointer;
    color: var(--kk-text-muted, GrayText);
  }
  .rendered {
    margin: 8px 0 2px;
    padding: 8px 10px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-input, Field);
    color: var(--kk-text-primary, CanvasText);
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    max-height: 320px;
    overflow: auto;
  }
</style>
