<!-- What each connected core uses (06-client, About): memory, active agents, processes,
     database size and uptime, from `core.stats`. Read when shown and on Refresh; nothing
     polls. -->
<script lang="ts">
  import type { CoreStatsResult } from '@kumikit/protocol';
  import type { Kernel } from '../app.svelte';
  import { formatBytes, formatUptime } from './about';

  let { kernel }: { kernel: Kernel } = $props();

  type Row = { stats?: CoreStatsResult; error?: string };
  let rows = $state.raw<Record<string, Row>>({});
  let loading = $state(false);
  const ready = $derived(kernel.connections.filter((c) => c.ready));

  async function load() {
    loading = true;
    const next: Record<string, Row> = {};
    await Promise.all(
      ready.map(async (c) => {
        try {
          next[c.id] = { stats: await c.call('core.stats', {}) };
        } catch (e) {
          next[c.id] = { error: e instanceof Error ? e.message : String(e) };
        }
      }),
    );
    rows = next;
    loading = false;
  }

  $effect(() => {
    // Again when a core connects or goes away.
    void ready.map((c) => c.id).join();
    void load();
  });
</script>

<section class="usage" aria-labelledby="kk-usage-heading">
  <div class="head">
    <h3 id="kk-usage-heading">Resource usage</h3>
    <button type="button" class="kk-button" disabled={loading} onclick={() => void load()}>{loading ? 'Reading…' : 'Refresh'}</button>
  </div>
  {#if ready.length === 0}
    <p class="hint">No core is connected.</p>
  {/if}
  {#each ready as c (c.id)}
    {@const row = rows[c.id]}
    <div class="core" role="group" aria-label={`Resource usage of ${c.label}`}>
      <div class="core-head">
        <strong>{c.label}</strong>
        {#if c.hello}<span class="hint">Core v{c.hello.core.version} on {c.hello.core.name}</span>{/if}
      </div>
      {#if row?.error}
        <p class="hint" role="alert">Cannot read: {row.error}</p>
      {:else if row?.stats}
        {@const s = row.stats}
        <dl>
          <div><dt>Core memory</dt><dd>{s.memoryBytes !== undefined ? formatBytes(s.memoryBytes) : 'Not reported'}</dd></div>
          <div><dt>Active agents</dt><dd>{s.activeTurns === 0 ? 'None' : `${s.activeTurns} running`}</dd></div>
          <div><dt>Processes</dt><dd>{s.processes}</dd></div>
          <div><dt>Database</dt><dd>{formatBytes(s.databaseBytes)}</dd></div>
          <div><dt>Uptime</dt><dd>{formatUptime(s.uptimeSeconds)}</dd></div>
        </dl>
      {:else}
        <p class="hint">Reading…</p>
      {/if}
    </div>
  {/each}
</section>

<style>
  .usage {
    display: grid;
    gap: 10px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  h3 {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-secondary, CanvasText);
  }
  .hint {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  .core {
    display: grid;
    gap: 10px;
    padding: 14px 16px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  .core-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: 8px;
  }
  dl {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(120px, 1fr));
    gap: 10px 16px;
    margin: 0;
  }
  dt {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
  }
  dd {
    margin: 2px 0 0;
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-base, medium);
    color: var(--kk-text-primary, CanvasText);
  }
</style>
