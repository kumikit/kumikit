<!-- Workspaces and their threads, with each thread's status (06-client, Sidebar). -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel }: { kernel: Kernel } = $props();
  const conn = $derived(kernel.connection);
  const statusText: Record<string, string> = {
    idle: 'idle',
    running: 'running',
    awaiting_approval: 'waiting for approval',
    failed: 'failed',
    archived: 'archived',
  };
</script>

<nav class="sidebar" aria-label="Workspaces">
  <div class="head">
    <span class="core" title={conn.hello ? `Core ${conn.hello.core.version} on ${conn.hello.core.name}` : undefined}>
      {conn.hello?.core.name ?? 'Local core'}
    </span>
    <button type="button" class="kk-button" onclick={() => kernel.openWorkspaceFolder()} disabled={!conn.ready}>Open folder…</button>
  </div>
  {#each conn.workspaces as ws (ws.id)}
    {@const open = kernel.tabs.some((t) => t.workspace === ws.id)}
    <section aria-label={ws.name}>
      <button type="button" class="ws" class:current={kernel.activeTab?.workspace === ws.id} title={ws.root} onclick={() => kernel.openTab(ws.id)}>
        {ws.name}
      </button>
      {#if open}
        <ul>
          {#each (conn.threads[ws.id] ?? []).filter((t) => t.status !== 'archived') as t (t.id)}
            <li>
              <button
                type="button"
                class="thread"
                class:current={kernel.activeThread?.thread === t.id}
                aria-current={kernel.activeThread?.thread === t.id ? 'page' : undefined}
                onclick={() => kernel.openTab(ws.id, t.id)}
              >
                <span class="kk-status {t.status}" aria-hidden="true"></span>
                <span class="title">{t.title ?? `Thread ${t.created.slice(0, 16).replace('T', ' ')}`}</span>
                <span class="kk-sr-only">, {statusText[t.status] ?? t.status}</span>
              </button>
            </li>
          {:else}
            <li class="kk-muted none">No threads yet</li>
          {/each}
        </ul>
      {/if}
    </section>
  {:else}
    <p class="kk-muted none">Open a project folder to start.</p>
  {/each}
  {#each kernel.slots.in('sidebar.section') as entry (entry)}
    <section aria-label={entry.contribution.title}>
      <h2>{entry.contribution.title}</h2>
      <SlotHost plugin={entry.plugin} contribution={entry.contribution} props={{ connection: kernel.pluginConnection() }} slots={kernel.slots} />
    </section>
  {/each}
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    gap: 8px;
    height: 100%;
    overflow: auto;
    padding: 8px;
    background: var(--kk-surface-panel, Canvas);
    border-right: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }
  .core {
    font-weight: var(--kk-font-weight-strong, bold);
  }
  h2 {
    margin: 4px 0;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0 0 0 8px;
  }
  .ws,
  .thread {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 4px 6px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: none;
    text-align: left;
    cursor: pointer;
  }
  .ws {
    font-weight: var(--kk-font-weight-strong, bold);
  }
  .ws:hover,
  .thread:hover {
    background: var(--kk-surface-hover, Canvas);
  }
  .current {
    background: var(--kk-surface-selected, Highlight);
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .none {
    padding: 4px 6px;
    margin: 0;
  }
</style>
