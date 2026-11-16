<!--
  Mounts one plugin contribution into an element it owns. A contribution that throws
  shows a placeholder and counts as a failure of its plugin (07-ui-plugins).
-->
<script lang="ts" generics="P">
  import type { Mountable } from '@kumikit/ui-sdk';
  import type { SlotRegistry } from '../slots/registry.svelte';

  let {
    plugin,
    contribution,
    props,
    slots,
    label,
  }: { plugin: string; contribution: Mountable<P>; props: P; slots: SlotRegistry; label?: string } = $props();

  let el: HTMLElement;
  let failed = $state<string | undefined>();

  $effect(() => {
    const p = props;
    failed = undefined;
    let unmount: (() => void) | void;
    try {
      unmount = contribution.mount(el, p);
    } catch (e) {
      failed = e instanceof Error ? e.message : String(e);
      slots.fail(plugin, e);
    }
    return () => {
      try {
        if (typeof unmount === 'function') unmount();
      } catch (e) {
        slots.fail(plugin, e);
      }
      el.replaceChildren();
    };
  });
</script>

<div class="host" bind:this={el} aria-label={label} hidden={failed !== undefined}></div>
{#if failed !== undefined}
  <div class="failed" role="alert">
    <strong>{plugin}</strong> could not show this part: {failed}
  </div>
{/if}

<style>
  .host {
    display: contents;
  }
  .failed {
    padding: 8px 12px;
    border: 1px dashed var(--kk-state-danger, CanvasText);
    border-radius: var(--kk-radius-small, 0);
    color: var(--kk-text-secondary, CanvasText);
  }
</style>
