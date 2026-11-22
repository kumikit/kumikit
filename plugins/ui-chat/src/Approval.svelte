<!-- An approval: the agent waits until it is answered here or on another client. -->
<script lang="ts">
  import type { RendererProps, TranscriptItem } from '@kumikit/ui-sdk';
  let { item, run }: { item: Extract<TranscriptItem, { kind: 'approval' }>; run: RendererProps['run'] } = $props();
  let sent = $state<string | undefined>();
  const resolved = $derived(item.resolution);
  const detailText = $derived.by(() => {
    const d = item.approval.detail;
    if (d === undefined || d === null) return undefined;
    return typeof d === 'string' ? d : JSON.stringify(d, null, 2);
  });
  const words: Record<string, string> = { allow: 'Allowed', allow_always: 'Always allowed', deny: 'Denied', cancelled: 'Cancelled' };

  function answer(decision: 'allow' | 'allow_always' | 'deny') {
    sent = decision;
    void run('approval.respond', { approval: item.approval.id, decision });
  }
</script>

<section class="kk-approval" class:open={!resolved} aria-label="Approval">
  <p class="summary"><strong>{resolved ? words[resolved] ?? resolved : 'Needs your approval'}:</strong> {item.approval.summary}</p>
  {#if detailText}<pre>{detailText}</pre>{/if}
  {#if !resolved}
    <div class="actions" role="group" aria-label="Answer">
      <button type="button" class="kk-button primary" onclick={() => answer('allow')} aria-pressed={sent === 'allow'}>Allow</button>
      <button type="button" class="kk-button" onclick={() => answer('allow_always')} aria-pressed={sent === 'allow_always'}>Always allow</button>
      <button type="button" class="kk-button danger" onclick={() => answer('deny')} aria-pressed={sent === 'deny'}>Deny</button>
    </div>
  {/if}
</section>

<style>
  .kk-approval {
    padding: 10px 14px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-left: 3px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    font-size: var(--kk-font-size-small, small);
  }
  .open {
    border-left-color: var(--kk-state-warning, Highlight);
    background: var(--kk-state-warning-tint, Canvas);
  }
  .summary {
    margin: 0;
  }
  pre {
    overflow: auto;
    max-height: 240px;
    margin: 8px 0 0;
    padding: 8px 10px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-code-background, Canvas);
    white-space: pre-wrap;
  }
  .actions {
    display: flex;
    gap: 6px;
    margin-top: 10px;
  }
</style>
