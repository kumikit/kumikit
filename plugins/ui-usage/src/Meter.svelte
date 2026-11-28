<!-- The thread header's usage meter: the plan limit window of the thread's provider that
     is closest to full (among those covering the thread's model). Hidden while the
     provider has reported none. A click opens the Agent usage dialog on that provider. -->
<script lang="ts">
  import type { Connection, PluginContext, ThreadAddress, ThreadView } from '@kumikit/ui-sdk';
  import { onDestroy } from 'svelte';

  import { fill } from './bar';
  import { level, percent, resetTime, shortLabel, tightest } from './format';
  import { acquire } from './store.svelte';
  import UsageDialog from './UsageDialog.svelte';

  let { ctx, address, connection }: { ctx: PluginContext; address: ThreadAddress; connection: Connection } = $props();

  // svelte-ignore state_referenced_locally
  const [store, release] = acquire(connection);
  onDestroy(release);

  let view = $state<ThreadView>();
  $effect(() => ctx.threads.watch(address, (v) => (view = v)));
  $effect(() => {
    if (connection.ready) void store.load().catch(() => {});
  });

  const provider = $derived(view?.thread?.provider);
  const model = $derived(view?.thread?.model);
  const snapshot = $derived(provider ? store.limits[provider] : undefined);
  const win = $derived(tightest(snapshot?.limits, model));
  const limited = $derived(view?.status === 'limited');
  const lvl = $derived(limited ? 'danger' : win ? level(win.usedPercent) : 'ok');
  const providerName = $derived(store.providers.find((p) => p.id === provider)?.name ?? provider ?? '');

  const tooltip = $derived.by(() => {
    const lines = (snapshot?.limits?.windows ?? []).map((w) => {
      const scope = w.group ? `${w.group} · ` : '';
      const reset = w.resetsAt ? `, resets ${resetTime(w.resetsAt)}` : '';
      return `${scope}${w.label}: ${percent(w.usedPercent)} used${reset}`;
    });
    return [`${providerName} plan limits`, ...lines, 'Click for Agent usage'].join('\n');
  });

  let open = $state(false);
  let button: HTMLButtonElement | undefined = $state();

  function close() {
    open = false;
    button?.focus();
  }
</script>

{#if win}
  <button
    bind:this={button}
    type="button"
    class="kk-usage-meter"
    data-level={lvl}
    aria-label={`${providerName}: ${win.label} limit ${percent(win.usedPercent)} used. Open Agent usage`}
    aria-haspopup="dialog"
    title={tooltip}
    onclick={() => (open = true)}
  >
    <span class="label">{shortLabel(win.label)}</span>
    <span class="track" aria-hidden="true"><span class="fill" use:fill={win.usedPercent}></span></span>
    <span class="pct">{percent(win.usedPercent)}</span>
  </button>
{/if}
{#if open}
  <UsageDialog {ctx} {store} initial={provider} onclose={close} />
{/if}

<style>
  .kk-usage-meter {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    height: 22px;
    padding: 0 8px;
    border: 1px solid var(--kk-border-subtle, ButtonBorder);
    border-radius: var(--kk-radius-medium, 0);
    background: transparent;
    color: var(--kk-text-secondary, ButtonText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    font-variant-numeric: tabular-nums;
    cursor: pointer;
  }
  .kk-usage-meter:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, ButtonText);
  }
  .kk-usage-meter:focus-visible {
    outline: 2px solid var(--kk-focus, Highlight);
    outline-offset: 1px;
  }
  .track {
    position: relative;
    width: 44px;
    height: 4px;
    overflow: hidden;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-border-subtle, GrayText);
  }
  .fill {
    position: absolute;
    inset: 0 auto 0 0;
    border-radius: inherit;
    background: var(--kk-text-secondary, CanvasText);
  }
  [data-level='warn'] .fill {
    background: var(--kk-state-warning, CanvasText);
  }
  [data-level='danger'] .fill {
    background: var(--kk-state-danger, CanvasText);
  }
  [data-level='danger'] .pct {
    color: var(--kk-state-danger, CanvasText);
  }
</style>
