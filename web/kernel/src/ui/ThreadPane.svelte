<!-- The active tab: its core's state, and its open thread, shown by the first
`thread.view` contribution that applies on that core. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel }: { kernel: Kernel } = $props();
  const tab = $derived(kernel.activeTab);
  const conn = $derived(tab ? kernel.connectionFor(tab.connection) : kernel.connection);
  const address = $derived(conn ? kernel.activeThread : undefined);
  const view = $derived(address ? kernel.slotsIn('thread.view', address.connection)[0] : undefined);
  const thread = $derived(address && conn ? (conn.threads[address.workspace] ?? []).find((t) => t.id === address.thread) : undefined);
  const slotProps = $derived(address ? { address, connection: kernel.pluginConnection(address.connection) } : undefined);
  const statusText: Record<string, string> = {
    idle: 'Idle',
    running: 'Running',
    awaiting_approval: 'Waiting for your approval',
    failed: 'Failed',
    archived: 'Archived',
  };
</script>

{#if tab && !conn}
  <div class="kk-empty">
    <p>This core was removed from this window.</p>
    <p><button type="button" class="kk-button" onclick={() => kernel.closeTab(kernel.activeIndex)}>Close the tab</button></p>
  </div>
{:else}
{#if conn && conn.status !== 'ready'}
  <div class="banner" role={conn.status === 'reconnecting' || conn.status === 'connecting' ? 'status' : 'alert'} data-core={conn.id}>
    {#if conn.status === 'mismatch'}
      <span>{conn.error}. This window and {conn.label} run different versions of Kumikit.</span>
      {#if conn.kind === 'local'}
        <button type="button" class="kk-button" onclick={() => kernel.commands.run('core.restart')}>Restart the core</button>
      {/if}
    {:else if conn.status === 'unauthorized'}
      <span>{conn.label} no longer accepts this computer ({conn.error}). Remove the core and add it again with a new pairing code.</span>
      <button type="button" class="kk-button" onclick={() => (kernel.main = { kind: 'cores' })}>Cores…</button>
    {:else if conn.status === 'certificate_changed'}
      <span>{conn.error}</span>
      <button type="button" class="kk-button" onclick={() => (kernel.main = { kind: 'cores' })}>Cores…</button>
    {:else}
      <span>Reconnecting to {conn.label}… {conn.error ?? ''}</span>
      <button type="button" class="kk-button" onclick={() => conn.reconnectNow()}>Reconnect now</button>
    {/if}
  </div>
{/if}
{#if !tab}
  <div class="kk-empty">
    <p>Open a project folder to start working with an agent.</p>
    <p><button type="button" class="kk-button primary" onclick={() => kernel.openWorkspaceFolder()}>Open folder…</button></p>
  </div>
{:else if !address || !slotProps}
  <div class="kk-empty">
    <p>No thread is open in this workspace.</p>
    <p><button type="button" class="kk-button primary" onclick={() => (kernel.dialog = { kind: 'new-thread' })}>New thread…</button></p>
  </div>
{:else}
  <section class="pane" aria-label={thread?.title ?? 'Thread'}>
    <header>
      <h1>{thread?.title ?? 'Thread'}</h1>
      {#if thread}
        <span class="status"><span class="kk-status {thread.status}" aria-hidden="true"></span>{statusText[thread.status] ?? thread.status}</span>
        <span class="kk-muted">{conn?.providers.find((p) => p.id === thread.provider)?.name ?? thread.provider} · {thread.model}</span>
      {/if}
      <span class="spacer"></span>
      {#each kernel.slotsIn('thread.header', address.connection) as entry (entry)}
        <SlotHost plugin={entry.plugin} contribution={entry.contribution} props={slotProps} slots={kernel.slots} />
      {/each}
      {#if thread && (thread.status === 'running' || thread.status === 'awaiting_approval')}
        <button type="button" class="kk-button" onclick={() => kernel.commands.run('thread.cancel')}>Cancel turn</button>
      {/if}
    </header>
    <div class="body">
      {#if view}
        {#key `${address.connection}/${address.workspace}/${address.thread}`}
          <SlotHost plugin={view.plugin} contribution={view.contribution} props={slotProps} slots={kernel.slots} label="Thread" />
        {/key}
      {:else}
        <div class="kk-empty">
          <p>No plugin shows threads. Enable a UI plugin that contributes a thread view in Manage plugins.</p>
        </div>
      {/if}
    </div>
  </section>
{/if}
{/if}

<style>
  .banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 12px;
    background: var(--kk-state-warning-tint, Mark);
    color: var(--kk-text-primary, MarkText);
  }
  .pane {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 8px 16px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    background: var(--kk-surface-panel, Canvas);
  }
  h1 {
    margin: 0;
    font-size: var(--kk-font-size-large, large);
  }
  .status {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }
  .spacer {
    flex: 1;
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
