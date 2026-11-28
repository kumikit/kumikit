<!-- The Overview tab: one row per enabled provider with its tightest plan limit window
     and its usage over the past 30 days, and the totals. -->
<script lang="ts">
  import type { UsageSummary } from '@kumikit/protocol';
  import type { PluginContext } from '@kumikit/ui-sdk';

  import { fill } from './bar';
  import { cost, level, percent, rowTokens, tightest, tokens, tzOffsetMinutes } from './format';
  import type { UsageStore } from './store.svelte';

  let { ctx, store, onopen }: { ctx: PluginContext; store: UsageStore; onopen: (provider: string) => void } = $props();

  let month = $state<UsageSummary>();

  $effect(() => {
    void store.version;
    let stale = false;
    store.connection.call('usage.summary', { days: 30, tzOffsetMinutes: tzOffsetMinutes() }).then(
      (m) => {
        if (!stale) month = m;
      },
      (e: unknown) => {
        if (!stale) ctx.notify(`Cannot read usage: ${e instanceof Error ? e.message : String(e)}`, 'danger');
      },
    );
    return () => {
      stale = true;
    };
  });

  const rows = $derived(
    store.providers.map((p) => {
      const models = (month?.models ?? []).filter((m) => m.provider === p.id);
      const costs = models.map((m) => m.cost).filter((c): c is number => c !== undefined);
      return {
        provider: p,
        window: tightest(store.limits[p.id]?.limits, undefined),
        tokens: models.reduce((n, m) => n + rowTokens(m), 0),
        cost: costs.length > 0 ? costs.reduce((a, b) => a + b, 0) : undefined,
      };
    }),
  );
</script>

{#if store.providers.length === 0}
  <p class="empty">No providers are enabled on this core.</p>
{:else}
  {#if month}
    <dl class="tiles">
      <div>
        <dt>Estimated cost, 30 days</dt>
        <dd>{cost(month.totals.cost)}</dd>
      </div>
      <div>
        <dt>Tokens</dt>
        <dd>{tokens(month.totals.inputTokens + month.totals.outputTokens)}</dd>
      </div>
      <div>
        <dt>Threads</dt>
        <dd>{month.threads}</dd>
      </div>
    </dl>
  {/if}
  <table class="providers">
    <caption class="sr-only">Providers</caption>
    <thead>
      <tr>
        <th scope="col">Provider</th>
        <th scope="col">Closest limit</th>
        <th scope="col" class="num">Cost</th>
        <th scope="col" class="num">Tokens</th>
      </tr>
    </thead>
    <tbody>
      {#each rows as r (r.provider.id)}
        <tr>
          <th scope="row">
            <button type="button" class="open" onclick={() => onopen(r.provider.id)}>
              {#if r.provider.icon}<img class="picon" src={r.provider.icon} alt="" aria-hidden="true" draggable="false" />{/if}
              {r.provider.name}
            </button>
          </th>
          <td>
            {#if r.window}
              <span class="limit" data-level={level(r.window.usedPercent)}>
                <span
                  class="track"
                  role="meter"
                  aria-label={`${r.provider.name}, ${r.window.label}`}
                  aria-valuemin={0}
                  aria-valuemax={100}
                  aria-valuenow={Math.round(r.window.usedPercent)}
                  aria-valuetext={`${percent(r.window.usedPercent)} used`}
                ><span class="fill" use:fill={r.window.usedPercent}></span></span>
                <span class="what">{r.window.label} · {percent(r.window.usedPercent)}</span>
              </span>
            {:else}
              <span class="none">Not reported</span>
            {/if}
          </td>
          <td class="num">{month ? cost(r.cost) : '…'}</td>
          <td class="num">{month ? tokens(r.tokens) : '…'}</td>
        </tr>
      {/each}
    </tbody>
  </table>
{/if}

<style>
  .tiles {
    display: grid;
    grid-template-columns: repeat(3, minmax(0, 1fr));
    gap: 12px;
    margin: 8px 0 18px;
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
  .providers {
    width: 100%;
    border-collapse: collapse;
    font-size: var(--kk-font-size-small, small);
  }
  .providers th,
  .providers td {
    padding: 7px 0;
    text-align: left;
    font-weight: var(--kk-font-weight-normal, normal);
  }
  .providers thead th {
    color: var(--kk-text-muted, GrayText);
  }
  .providers tbody tr + tr {
    border-top: 1px solid var(--kk-border-subtle, GrayText);
  }
  .providers .num {
    width: 90px;
    text-align: right;
    font-variant-numeric: tabular-nums;
  }
  .open {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 0;
    border: 0;
    background: none;
    color: var(--kk-text-primary, ButtonText);
    font: inherit;
    cursor: pointer;
  }
  .open:hover {
    color: var(--kk-text-link, LinkText);
  }
  .open:focus-visible {
    outline: 2px solid var(--kk-focus, Highlight);
    outline-offset: 1px;
  }
  .picon {
    width: 14px;
    height: 14px;
    object-fit: contain;
  }
  .limit {
    display: inline-flex;
    align-items: center;
    gap: 8px;
  }
  .track {
    position: relative;
    width: 80px;
    height: 5px;
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
  .what,
  .none {
    color: var(--kk-text-secondary, CanvasText);
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
