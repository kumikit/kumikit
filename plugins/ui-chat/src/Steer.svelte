<!-- A steering message taken by the agent mid-turn (ADR 0028), shown inline in the
     transcript right where the agent received it. -->
<script lang="ts">
  let { text }: { text: string } = $props();
</script>

<div class="kk-steer" role="note" aria-label="Steered message">
  <span class="badge">Steered</span>
  <p>{text}</p>
</div>

<style>
  .kk-steer {
    align-self: flex-end;
    max-width: 82%;
    margin-left: auto;
    padding: 8px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-primary, CanvasText);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .badge {
    align-self: flex-start;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-accent, Highlight);
  }
  p {
    margin: 0;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
  }
</style>
