<!-- The turn's reasoning, in one box under its tool calls. Collapsed by default, like a tool
     call; when open it has a limited height and scrolls, and while it is open the newest
     text stays in view as it streams. -->
<script lang="ts">
  let { text }: { text: string } = $props();

  let open = $state(false);
  let body = $state<HTMLElement | undefined>();

  $effect(() => {
    void text;
    if (open && body) body.scrollTop = body.scrollHeight;
  });
</script>

<details class="thinking" ontoggle={(e) => (open = (e.currentTarget as HTMLDetailsElement).open)}>
  <summary><span class="title">Thinking...</span><svg class="chevron" width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><path d="M6 3.5 10.5 8 6 12.5" /></svg></summary>
  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
  <!-- A scrollable text region must be reachable by keyboard (WCAG 2.1.1), so it takes focus. -->
  <div class="thinking-body" bind:this={body} tabindex="0" role="region" aria-label="Thinking text">
    <p>{text}</p>
  </div>
</details>

<style>
  .thinking {
    padding: 5px 10px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  summary {
    display: flex;
    align-items: center;
    gap: 6px;
    cursor: pointer;
    color: var(--kk-text-muted, GrayText);
    list-style: none;
    user-select: none;
  }
  summary::-webkit-details-marker {
    display: none;
  }
  summary:hover {
    color: var(--kk-text-secondary, CanvasText);
  }
  .title {
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .chevron {
    flex: none;
    color: var(--kk-text-muted, GrayText);
    transition: transform var(--kk-motion-duration-short, 0ms) var(--kk-motion-ease-out, ease-out);
  }
  .thinking[open] > summary .chevron {
    transform: rotate(90deg);
  }
  @media (prefers-reduced-motion: reduce) {
    .chevron {
      transition: none;
    }
  }
  .thinking-body {
    max-height: 240px;
    overflow-y: auto;
    margin-top: 6px;
    padding-right: 4px;
  }
  .thinking-body:focus-visible {
    outline: 2px solid var(--kk-focus, Highlight);
    outline-offset: 2px;
  }
  p {
    margin: 0 0 2px;
    white-space: pre-wrap;
  }
</style>
