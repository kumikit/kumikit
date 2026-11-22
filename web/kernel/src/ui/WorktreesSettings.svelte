<!-- Managed Git worktrees stored under `<homeDir>/worktrees/` on the active core:
     lists each worktree with its disk usage and linked thread count, and lets the user
     delete unused worktrees cleanly. -->
<script lang="ts">
  import type { WorktreeEntry } from '@kumikit/protocol';
  import { untrack } from 'svelte';
  import type { Kernel } from '../app.svelte';
  import Icon from './Icon.svelte';

  let { kernel }: { kernel: Kernel } = $props();

  const conn = $derived(kernel.connection);

  let root = $state('');
  let worktrees = $state<WorktreeEntry[]>([]);
  let totalBytes = $state(0);
  let loading = $state(false);
  let error = $state<string | null>(null);
  let deletingPath = $state<string | null>(null);
  let loadSeq = 0;

  function formatBytes(bytes: number): string {
    if (bytes < 1024) return `${bytes} B`;
    const units = ['KB', 'MB', 'GB', 'TB'];
    let value = bytes / 1024;
    let idx = 0;
    while (value >= 1024 && idx < units.length - 1) {
      value /= 1024;
      idx += 1;
    }
    const decimals = value >= 100 ? 0 : value >= 10 ? 1 : 2;
    return `${value.toFixed(decimals)} ${units[idx]}`;
  }

  async function refresh() {
    const c = conn;
    if (!c.ready) return;
    const seq = ++loadSeq;
    loading = true;
    error = null;
    try {
      const res = await c.call('core.worktrees.list', {});
      if (seq !== loadSeq) return;
      root = res.root;
      worktrees = res.worktrees;
      totalBytes = res.totalBytes;
    } catch (e) {
      if (seq !== loadSeq) return;
      error = e instanceof Error ? e.message : String(e);
    } finally {
      if (seq === loadSeq) loading = false;
    }
  }

  $effect(() => {
    const c = conn;
    const ready = c.ready;
    if (!ready) return;
    untrack(() => {
      void refresh();
    });
    return () => {
      loadSeq += 1;
    };
  });

  async function remove(entry: WorktreeEntry) {
    if (entry.inUse || deletingPath) return;
    deletingPath = entry.path;
    error = null;
    try {
      await conn.call('core.worktrees.delete', { path: entry.path });
      kernel.notify(`Deleted worktree ${entry.branch ?? entry.name}`, 'info');
      await refresh();
    } catch (e) {
      error = e instanceof Error ? e.message : String(e);
    } finally {
      deletingPath = null;
    }
  }
</script>

<div class="worktrees">
  <div class="head">
    <div class="intro">
      <h2>Worktrees</h2>
      <p class="sub">
        Managed Git worktrees on <strong>{conn.label}</strong>. Deleting a worktree thread also removes its folder when no other thread uses it.
      </p>
    </div>
    <button
      type="button"
      class="kk-button"
      disabled={!conn.ready || loading}
      onclick={() => void refresh()}
    >
      {loading ? 'Scanning…' : 'Refresh'}
    </button>
  </div>

  {#if root}
    <div class="summary" aria-live="polite">
      <div class="summary-stats">
        <span class="stat-strong">
          {worktrees.length} {worktrees.length === 1 ? 'worktree' : 'worktrees'}
        </span>
        <span class="dot" aria-hidden="true">·</span>
        <span class="stat-strong">{formatBytes(totalBytes)}</span>
      </div>
      <code class="root-path" title={root}>{root}</code>
    </div>
  {/if}

  {#if error}
    <p class="err" role="alert">{error}</p>
  {/if}

  {#if !loading && worktrees.length === 0 && !error}
    <div class="empty">
      <p class="empty-title">No managed worktrees</p>
      <p class="empty-hint">
        Create a worktree from the branch menu in any Git workspace composer to isolate changes in its own folder.
      </p>
    </div>
  {:else if worktrees.length > 0}
    <ul class="list" aria-label="Managed worktrees">
      {#each worktrees as entry (entry.path)}
        {@const label = entry.branch ?? entry.name}
        {@const busy = deletingPath === entry.path}
        <li class="card">
          <div class="card-main">
            <div class="card-title-row">
              <span class="wt-icon" aria-hidden="true">
                <Icon name="worktree" size={14} />
              </span>
              <span class="wt-name">{label}</span>
              <span class="chip">{entry.repo}</span>
              {#if entry.inUse}
                <span class="chip chip-running">Running</span>
              {:else if entry.threadCount > 0}
                <span class="chip">
                  {entry.threadCount} {entry.threadCount === 1 ? 'thread' : 'threads'}
                </span>
              {:else}
                <span class="chip">Unused</span>
              {/if}
            </div>
            <code class="wt-path" title={entry.path}>{entry.path}</code>
          </div>
          <div class="card-actions">
            <span class="wt-size">{formatBytes(entry.sizeBytes)}</span>
            <button
              type="button"
              class="kk-button danger-btn"
              aria-label={`Delete worktree ${label}`}
              title={entry.inUse ? 'Stop the running turn before deleting this worktree' : `Delete ${label}`}
              disabled={entry.inUse || deletingPath !== null}
              onclick={() => void remove(entry)}
            >
              <Icon name="trash" size={13} />
              <span>{busy ? 'Deleting…' : 'Delete'}</span>
            </button>
          </div>
        </li>
      {/each}
    </ul>
  {/if}
</div>

<style>
  .worktrees {
    display: grid;
    gap: 16px;
    max-width: 640px;
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 12px;
  }
  .intro {
    display: grid;
    gap: 4px;
    min-width: 0;
  }
  h2 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
  }
  .sub {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .summary {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    justify-content: space-between;
    gap: 8px 14px;
    padding: 10px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    font-size: var(--kk-font-size-small, small);
  }
  .summary-stats {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .stat-strong {
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
    font-variant-numeric: tabular-nums;
  }
  .dot {
    color: var(--kk-text-secondary, CanvasText);
  }
  .root-path {
    font-family: var(--kk-font-mono, monospace);
    color: var(--kk-text-secondary, CanvasText);
    word-break: break-all;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 8px;
  }
  .card {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
  }
  .card-main {
    display: grid;
    gap: 4px;
    min-width: 0;
  }
  .card-title-row {
    display: flex;
    flex-wrap: wrap;
    align-items: center;
    gap: 6px;
    min-width: 0;
  }
  .wt-icon {
    display: inline-flex;
    color: var(--kk-accent, Highlight);
  }
  .wt-name {
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip {
    padding: 1px 7px;
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    flex: none;
  }
  .chip-running {
    color: var(--kk-state-warning, CanvasText);
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .wt-path {
    font-family: var(--kk-font-mono, monospace);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
    word-break: break-all;
  }
  .card-actions {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: none;
  }
  .wt-size {
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
    font-variant-numeric: tabular-nums;
    white-space: nowrap;
  }
  .danger-btn {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    color: var(--kk-state-danger, CanvasText);
  }
  .danger-btn:hover:not(:disabled) {
    border-color: var(--kk-state-danger, CanvasText);
  }
  .empty {
    padding: 20px 16px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-medium, 0);
    background: var(--kk-surface-card, Canvas);
    display: grid;
    gap: 4px;
  }
  .empty-title {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
  }
  .empty-hint {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .err {
    margin: 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-state-danger, CanvasText);
  }
</style>
