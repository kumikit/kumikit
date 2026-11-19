<!-- Every core's workspaces and their threads, grouped by core, with each thread's status
(06-client, Sidebar). -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import type { CoreConnection } from '../connections/connection.svelte';
  import { untrack } from 'svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel }: { kernel: Kernel } = $props();
  // Sidebar sections get the active core's connection. The props change only when that
  // core does: a new object on every render would remount every section on any change.
  const activeCore = $derived(kernel.connection.id);
  const sectionProps = $derived.by(() => {
    void activeCore;
    return untrack(() => ({ connection: kernel.pluginConnection() }));
  });
  const statusText: Record<string, string> = {
    idle: 'idle',
    running: 'running',
    awaiting_approval: 'waiting for approval',
    failed: 'failed',
    archived: 'archived',
  };
  const connText: Record<string, string> = {
    connecting: 'connecting',
    reconnecting: 'reconnecting',
    mismatch: 'different version',
    unauthorized: 'needs pairing',
    certificate_changed: 'certificate changed',
  };
  const coreTitle = (c: CoreConnection) => (c.hello ? `Core ${c.hello.core.version} on ${c.hello.core.name}` : c.label);
</script>

<nav class="sidebar" aria-label="Workspaces">
  {#each kernel.connections as conn (conn.id)}
    <section class="core" aria-label={conn.label} data-core={conn.id}>
      <div class="head">
        <span class="name" title={coreTitle(conn)}>{conn.label}</span>
        {#if conn.status !== 'ready'}
          <span class="state {conn.status}" role="status">{connText[conn.status] ?? conn.status}</span>
        {/if}
        <button type="button" class="kk-button" onclick={() => kernel.openWorkspaceFolder(conn.id)} disabled={!conn.ready}>Open folder…</button>
      </div>
      {#each conn.workspaces as ws (ws.id)}
        {@const open = kernel.tabs.some((t) => t.connection === conn.id && t.workspace === ws.id)}
        {@const current = kernel.activeTab?.connection === conn.id && kernel.activeTab?.workspace === ws.id}
        <section aria-label={ws.name}>
          <button type="button" class="ws" class:current title={ws.root} onclick={() => kernel.openTab(conn.id, ws.id)}>
            {ws.name}
          </button>
          {#if open}
            <ul>
              {#each (conn.threads[ws.id] ?? []).filter((t) => t.status !== 'archived') as t (t.id)}
                {@const here = current && kernel.activeThread?.thread === t.id}
                <li>
                  <button
                    type="button"
                    class="thread"
                    class:current={here}
                    aria-current={here ? 'page' : undefined}
                    onclick={() => kernel.openTab(conn.id, ws.id, t.id)}
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
        {#if conn.ready}<p class="kk-muted none">Open a project folder to start.</p>{/if}
      {/each}
    </section>
  {/each}
  {#if kernel.platform.profiles.editable}
    <button type="button" class="kk-button add" onclick={() => (kernel.main = { kind: 'cores' })}>Add a core…</button>
  {/if}
  {#each kernel.slotsIn('sidebar.section') as entry (entry)}
    <section aria-label={entry.contribution.title}>
      <h2>{entry.contribution.title}</h2>
      <SlotHost plugin={entry.plugin} contribution={entry.contribution} props={sectionProps} slots={kernel.slots} />
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
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .core + .core {
    padding-top: 8px;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .name {
    flex: 1;
    font-weight: var(--kk-font-weight-strong, bold);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .state {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .add {
    align-self: flex-start;
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
