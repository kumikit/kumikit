<!-- A provider's plan limits: the account and plan, then one bar per window, grouped as
     the provider grouped them. Every bar shows the share used. -->
<script lang="ts">
  import type { ProviderInfo, ProviderLimits } from '@kumikit/protocol';
  import type { PluginContext } from '@kumikit/ui-sdk';

  import { fill } from './bar';
  import { level, percent, resetTime } from './format';

  let {
    ctx,
    provider,
    snapshot,
    refreshing,
  }: { ctx: PluginContext; provider: ProviderInfo; snapshot: ProviderLimits | undefined; refreshing: boolean } = $props();

  const limits = $derived(snapshot?.limits);
  const groups = $derived.by(() => {
    const out: { name: string | undefined; windows: NonNullable<typeof limits>['windows'] }[] = [];
    for (const w of limits?.windows ?? []) {
      const last = out[out.length - 1];
      if (last && last.name === w.group) last.windows.push(w);
      else out.push({ name: w.group, windows: [w] });
    }
    return out;
  });

  async function copyAccountLink(url: string) {
    // The app does not open web pages; the link is copied for the user's browser.
    try {
      await navigator.clipboard.writeText(url);
      ctx.notify('Account link copied', 'success');
    } catch {
      ctx.notify(url, 'info');
    }
  }
</script>

<section class="limits" aria-label={`${provider.name} plan limits`} aria-busy={refreshing}>
  {#if limits && (limits.account || limits.plan)}
    <p class="account">
      {#if limits.account}<span class="who">{limits.account}</span>{/if}
      {#if limits.plan}<span class="plan">{limits.plan}</span>{/if}
    </p>
  {/if}
  {#if limits && limits.windows.length > 0}
    {#each groups as g, gi (gi)}
      {#if g.name}<h4>{g.name}</h4>{/if}
      <div class="windows">
        {#each g.windows as w (w.id)}
          <div class="window" data-level={level(w.usedPercent)}>
            <div class="row">
              <span class="name">{w.label}</span>
              <span class="used">{percent(w.usedPercent)} used</span>
            </div>
            <div
              class="track"
              role="meter"
              aria-label={`${g.name ? `${g.name}, ` : ''}${w.label}`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(w.usedPercent)}
              aria-valuetext={`${percent(w.usedPercent)} used`}
            >
              <span class="fill" use:fill={w.usedPercent}></span>
            </div>
            {#if w.resetsAt}
              <span class="reset" title={new Date(w.resetsAt).toLocaleString('en-US')}>Resets {resetTime(w.resetsAt)}</span>
            {/if}
          </div>
        {/each}
      </div>
    {/each}
  {:else if snapshot?.observed}
    <p class="empty">This account has no plan limits to show.</p>
  {:else if provider.readsLimits}
    <p class="empty">No plan limits yet. Press Refresh to read them, or run a turn.</p>
  {:else}
    <p class="empty">This provider hasn't reported plan limits. Some agents report them while they run.</p>
  {/if}
  {#if limits?.accountUrl}
    <p class="links">
      <button type="button" class="link" onclick={() => copyAccountLink(limits.accountUrl!)}>Copy account link</button>
    </p>
  {/if}
</section>

<style>
  .account {
    display: flex;
    flex-direction: column;
    gap: 2px;
    margin: 0 0 14px;
  }
  .plan {
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  h4 {
    margin: 14px 0 6px;
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .windows {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
    gap: 14px 24px;
  }
  .row {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    margin-bottom: 6px;
    font-size: var(--kk-font-size-small, small);
  }
  .used {
    font-variant-numeric: tabular-nums;
  }
  .track {
    position: relative;
    height: 6px;
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
  .reset {
    display: block;
    margin-top: 6px;
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .empty {
    margin: 0;
    color: var(--kk-text-muted, GrayText);
    font-size: var(--kk-font-size-small, small);
  }
  .links {
    margin: 12px 0 0;
    text-align: right;
  }
  .link {
    padding: 0;
    border: 0;
    background: none;
    color: var(--kk-text-link, LinkText);
    font: inherit;
    font-size: var(--kk-font-size-small, small);
    cursor: pointer;
  }
  .link:focus-visible {
    outline: 2px solid var(--kk-focus, Highlight);
    outline-offset: 1px;
  }
</style>
