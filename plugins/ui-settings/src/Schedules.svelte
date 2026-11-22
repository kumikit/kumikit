<!-- Schedules on the active tab's core (03-kernel, Schedules): what runs when, and the
     controls to run one now, pause, resume or remove it. Reads `schedule.list` when shown
     and after each change; nothing polls. -->
<script lang="ts">
  import type { ScheduleInfo } from '@kumikit/protocol';
  import type { PluginContext } from '@kumikit/ui-sdk';

  let { ctx }: { ctx: PluginContext } = $props();
  let schedules = $state.raw<ScheduleInfo[]>([]);
  let problem = $state<string | undefined>();
  let busy = $state(false);

  const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
  const when = (t: string | undefined) => (t ? new Date(t).toLocaleString() : '—');
  const outcome: Record<string, string> = { ran: 'ran', failed: 'failed', missed: 'missed (the core was not running)', skipped: 'skipped (the last run was still going)' };

  function core() {
    if (!ctx.connection) throw new Error('no core is connected');
    return ctx.connection;
  }

  async function load() {
    try {
      schedules = (await core().call('schedule.list', {})).schedules;
      problem = undefined;
    } catch (e) {
      problem = message(e);
    }
  }

  async function act(fn: () => Promise<unknown>, done?: string) {
    busy = true;
    try {
      await fn();
      if (done) ctx.notify(done);
    } catch (e) {
      ctx.notify(message(e));
    } finally {
      busy = false;
      await load();
    }
  }

  $effect(() => {
    void load();
  });
</script>

<div class="kk-schedules">
  <div class="intro">
    <h2>Schedules</h2>
    <p class="kk-muted">Plugins that run on a schedule, on this core. They run only while the core runs.</p>
  </div>
  {#if problem}
    <p class="error" role="alert">Cannot read the schedules: {problem}</p>
  {:else if schedules.length === 0}
    <div class="empty-card">
      <p class="kk-muted">Nothing is scheduled.</p>
    </div>
  {:else}
    <div class="table-wrap">
      <table>
        <thead>
          <tr><th scope="col">What</th><th scope="col">When</th><th scope="col">Next run</th><th scope="col">Last run</th><th scope="col" aria-label="Actions"></th></tr>
        </thead>
        <tbody>
          {#each schedules as s (`${s.plugin}/${s.key}`)}
            <tr class:paused={!s.enabled}>
              <td>{s.description}<br /><span class="kk-muted">{s.plugin}</span></td>
              <td><code>{s.cron}</code> <span class="kk-muted">{s.timeZone ?? 'local time'}</span></td>
              <td>{s.enabled ? when(s.nextRun) : 'paused'}</td>
              <td>{when(s.lastRun)}{#if s.lastOutcome}<br /><span class="kk-muted">{outcome[s.lastOutcome] ?? s.lastOutcome}</span>{/if}</td>
              <td class="actions">
                <button type="button" class="kk-button" disabled={busy} onclick={() => act(() => core().call('schedule.run', { plugin: s.plugin, key: s.key }), 'Started')}>Run now</button>
                <button type="button" class="kk-button" disabled={busy} onclick={() => act(() => core().call('schedule.setEnabled', { plugin: s.plugin, key: s.key, enabled: !s.enabled }))}>{s.enabled ? 'Pause' : 'Resume'}</button>
                {#if s.source === 'plugin'}
                  <button type="button" class="kk-button danger" disabled={busy} onclick={() => act(() => core().call('schedule.delete', { plugin: s.plugin, key: s.key }))}>Remove</button>
                {/if}
              </td>
            </tr>
          {/each}
        </tbody>
      </table>
    </div>
  {/if}
</div>

<style>
  .kk-schedules {
    display: grid;
    gap: 14px;
  }
  .intro {
    display: grid;
    gap: 4px;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
  }
  .intro p {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
  }
  .empty-card {
    padding: 16px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  .empty-card p {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
  }
  .table-wrap {
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    overflow: auto;
  }
  table {
    border-collapse: collapse;
    width: 100%;
    font-size: var(--kk-font-size-small, small);
  }
  th {
    color: var(--kk-text-muted, GrayText);
    font-weight: var(--kk-font-weight-strong, bold);
    background: var(--kk-surface-frame, Canvas);
  }
  th,
  td {
    text-align: left;
    vertical-align: top;
    padding: 8px 10px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
  }
  tbody tr:last-child td {
    border-bottom: 0;
  }
  .paused {
    color: var(--kk-text-muted, GrayText);
  }
  .actions {
    display: flex;
    gap: 6px;
    flex-wrap: wrap;
  }
  .error {
    margin: 0;
    color: var(--kk-state-danger, CanvasText);
    font-size: var(--kk-font-size-small, small);
  }
</style>
