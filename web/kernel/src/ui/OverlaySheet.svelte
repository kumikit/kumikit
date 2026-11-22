<!-- Modern overlay sheet covering most of the app window (used for Plugins, Settings and Cores):
     dims the workspace underneath without unmounting it, closes via its × button, clicking
     outside on the backdrop, or pressing Escape (when no nested dialog is open). -->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';

  let {
    title,
    subtitle,
    onclose,
    hasNestedDialog = false,
    children,
  }: {
    title: string;
    subtitle?: string;
    onclose: () => void;
    hasNestedDialog?: boolean;
    children: Snippet;
  } = $props();

  let sheet: HTMLElement | undefined = $state();

  function onKey(e: KeyboardEvent) {
    if (isComposing(e) || hasNestedDialog) return;
    if (e.key === 'Escape') {
      // Do not close the overlay if an inner confirmation dialog inside the sheet is open.
      if (sheet?.querySelector('[role="dialog"]')) return;
      e.preventDefault();
      onclose();
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="scrim" data-overlay-scrim role="presentation" onclick={onclose}></div>
<div
  bind:this={sheet}
  class="sheet"
  role="region"
  aria-label={title}
  data-overlay={title.toLowerCase()}
>
  <header class="head">
    <div class="titles">
      <h1 class="title">{title}</h1>
      {#if subtitle}<span class="sub">{subtitle}</span>{/if}
    </div>
    <button
      type="button"
      class="kk-icon-btn close-btn"
      aria-label="Close"
      title="Close (Esc)"
      onclick={onclose}
    >
      <Icon name="x" size={15} />
    </button>
  </header>
  <div class="body">
    {@render children()}
  </div>
</div>

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 20;
    background: var(--kk-surface-overlay, Canvas);
    opacity: var(--kk-overlay-opacity, 0.6);
  }
  .sheet {
    position: fixed;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    z-index: 21;
    width: min(1060px, 92vw);
    height: min(84vh, 800px);
    display: flex;
    flex-direction: column;
    overflow: hidden;
    border: 1px solid var(--kk-border-strong, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-panel, Canvas);
    box-shadow: var(--kk-shadow-overlay, none);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    height: 44px;
    padding: 0 14px 0 20px;
    background: var(--kk-surface-frame, Canvas);
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    flex: none;
  }
  .titles {
    display: flex;
    align-items: baseline;
    gap: 10px;
    min-width: 0;
  }
  .title {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
  }
  .sub {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close-btn {
    width: 28px;
    height: 28px;
  }
  .body {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    display: flex;
    flex-direction: column;
  }
</style>
