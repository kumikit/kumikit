<!-- The active tab: its open thread, shown by the first `thread.view` contribution. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel }: { kernel: Kernel } = $props();
  const tab = $derived(kernel.activeTab);
  const address = $derived(kernel.activeThread);
  const view = $derived(address ? kernel.slots.in('thread.view')[0] : undefined);
  const thread = $derived(address ? (kernel.connection.threads[address.workspace] ?? []).find((t) => t.id === address.thread) : undefined);
  const slotProps = $derived(address ? { address, connection: kernel.pluginConnection() } : undefined);
  const statusText: Record<string, string> = {
    idle: 'Idle',
    running: 'Running',
    awaiting_approval: 'Waiting for your approval',
    failed: 'Failed',
    archived: 'Archived',
  };
</script>

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
        <span class="kk-muted">{kernel.connection.providers.find((p) => p.id === thread.provider)?.name ?? thread.provider} · {thread.model}</span>
      {/if}
      <span class="spacer"></span>
      {#each kernel.slots.in('thread.header') as entry (entry)}
        <SlotHost plugin={entry.plugin} contribution={entry.contribution} props={slotProps} slots={kernel.slots} />
      {/each}
      {#if thread && (thread.status === 'running' || thread.status === 'awaiting_approval')}
        <button type="button" class="kk-button" onclick={() => kernel.commands.run('thread.cancel')}>Cancel turn</button>
      {/if}
    </header>
    <div class="body">
      {#if view}
        {#key `${address.workspace}/${address.thread}`}
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

<style>
  .pane {
    display: flex;
    flex-direction: column;
    height: 100%;
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
