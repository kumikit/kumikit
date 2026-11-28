<!-- One provider's tab: its plan limits, its usage over the past 30 days by model, and a
     heatmap of its daily tokens. -->
<script lang="ts">
  import type { ProviderInfo, UsageSummary } from '@kumikit/protocol';
  import type { PluginContext } from '@kumikit/ui-sdk';

  import { checkedTime, cost, rowTokens, tokens, tzOffsetMinutes } from './format';
  import Heatmap from './Heatmap.svelte';
  import Limits from './Limits.svelte';
  import type { UsageStore } from './store.svelte';

  let { ctx, store, provider }: { ctx: PluginContext; store: UsageStore; provider: ProviderInfo } = $props();

  /** Days the heatmap covers: 20 weeks. */
  const ACTIVITY_DAYS = 140;

  let month = $state<UsageSummary>();
  let activity = $state<UsageSummary>();
  let failed = $state(false);

  const snapshot = $derived(store.limits[provider.id]);

  $effect(() => {
    void store.version;
    const id = provider.id;
    const tz = tzOffsetMinutes();
    let stale = false;
    Promise.all([
      store.connection.call('usage.summary', { provider: id, days: 30, tzOffsetMinutes: tz }),
      store.connection.call('usage.summary', { provider: id, days: ACTIVITY_DAYS, tzOffsetMinutes: tz }),
    ]).then(
      ([m, a]) => {
        if (stale) return;
        month = m;
        activity = a;
        failed = false;
      },
      (e: unknown) => {
        if (stale) return;
        failed = true;
        ctx.notify(`Cannot read usage: ${e instanceof Error ? e.message : String(e)}`, 'danger');
      },
    );
    return () => {
      stale = true;
    };
  });

  const source = $derived(
    provider.readsLimits
      ? 'Reported by the agent while it runs, and read on demand with Refresh.'
      : 'Reported by the agent while it runs.',
  );
</script>

<section class="head">
  {#if provider.icon}
    <img class="picon" src={provider.icon} alt="" aria-hidden="true" draggable="false" />
  {/if}
  <h3>{provider.name}</h3>
  <span class="spacer"></span>
  {#if snapshot?.observed}
    <span class="checked" title={`${source} Checked ${new Date(snapshot.observed).toLocaleString('en-US')}.`}>
      Checked {checkedTime(snapshot.observed)}
    </span>
  {/if}
</section>

<Limits {ctx} {provider} {snapshot} refreshing={store.refreshing[provider.id] === true} />

<section class="stats" aria-labelledby={`stats-${provider.id}`}>
  <h3 id={`stats-${provider.id}`}>Past 30 days</h3>
  {#if month}
    <dl class="tiles">
      <div>
        <dt>Estimated cost</dt>
        <dd>{cost(month.totals.cost)}</dd>
      </div>
      <div>
        <dt>Tokens</dt>
        <dd>{tokens(month.totals.inputTokens + month.totals.outputTokens)}</dd>
      </div>
      <div>
        <dt>Cached input</dt>
        <dd>{tokens(month.totals.cachedInputTokens)}</dd>
      </div>
      <div>
        <dt>Threads</dt>
        <dd>{month.threads}</dd>
      </div>
    </dl>
    {#if month.models.length > 0}
      <table class="models">
        <caption class="sr-only">Usage by model</caption>
        <thead>
          <tr><th scope="col">Model</th><th scope="col" class="num">Cost</th><th scope="col" class="num">Tokens</th></tr>
        </thead>
        <tbody>
          {#each month.models as row (row.provider + '/' + row.model)}
            <tr>
              <th scope="row">{row.model}</th>
              <td class="num">{cost(row.cost)}</td>
              <td class="num">{tokens(rowTokens(row))}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="empty">No turns in the past 30 days.</p>
    {/if}
  {:else if failed}
    <p class="empty">Usage could not be read.</p>
  {:else}
    <p class="empty">Reading usage…</p>
  {/if}
</section>

{#if activity}
  <Heatmap summary={activity} />
{/if}

<style>
  .head {
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 8px 0 12px;
  }
  .picon {
    width: 20px;
    height: 20px;
    object-fit: contain;
  }
  h3 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .spacer {
    flex: 1;
  }
  .checked {
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  .stats {
    margin-top: 20px;
  }
  .tiles {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 12px;
    margin: 10px 0 14px;
  }
  dt {
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
  dd {
    margin: 2px 0 0;
    font-size: var(--kk-font-size-large, large);
    font-weight: var(--kk-font-weight-strong, bold);
    font-variant-numeric: tabular-nums;
  }
  .models {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--kk-font-size-small, small);
  }
  .models th,
  .models td {
    padding: 6px 0;
    text-align: left;
    font-weight: var(--kk-font-weight-normal, normal);
  }
  .models thead th {
    color: var(--kk-text-muted, GrayText);
  }
  .models tbody th {
    font-family: var(--kk-font-mono, monospace);
  }
  .models .num {
    width: 90px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .models tbody tr + tr {
    border-top: 1px solid var(--kk-border-subtle, GrayText);
  }
  .empty {
    color: var(--kk-text-muted, GrayText);
    font-size: var(--kk-font-size-small, small);
  }
  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
    white-space: nowrap;
  }
</style>
