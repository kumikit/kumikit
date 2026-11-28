<!-- One workspace pane: its core's connection banner and its open thread, rendered by
     the first `thread.view` contribution that applies on that core. -->
<script lang="ts">
  import type { Kernel } from '../app.svelte';
  import { clampTabIndex, type LayoutLeaf } from '../layout/layout';
  import Icon from './Icon.svelte';
  import NewThreadComposer from './NewThreadComposer.svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel, pane }: { kernel: Kernel; pane?: LayoutLeaf } = $props();

  const target = $derived(pane ?? kernel.activePane);
  const tabIndex = $derived(clampTabIndex(target.tabs, target.activeTab));
  const tab = $derived(target.tabs[tabIndex]);
  const conn = $derived(tab ? kernel.connectionFor(tab.connection) : kernel.connection);
  const address = $derived(
    conn && tab?.thread ? { connection: tab.connection, workspace: tab.workspace, thread: tab.thread } : undefined,
  );
  const view = $derived(address ? kernel.slotsIn('thread.view', address.connection)[0] : undefined);
  const thread = $derived(address && conn ? (conn.threads[address.workspace] ?? []).find((t) => t.id === address.thread) : undefined);
  const slotProps = $derived(address ? { address, connection: kernel.pluginConnection(address.connection) } : undefined);
  const ws = $derived(tab && conn ? conn.workspaces.find((w) => w.id === tab.workspace) : undefined);
  const wsName = $derived(ws?.name);
  const isChat = $derived(ws?.kind === 'chat');

  // The registries' hosts, from their list (a local read: nothing is fetched).
  const registryHosts = $derived(
    [...new Set([...(kernel.registries.clientRegistries ?? []), ...(kernel.registries.coreRegistries ?? []).map((r) => r.url)].map(host))],
  );
  function host(url: string): string {
    try {
      return new URL(url).host || url;
    } catch {
      return url;
    }
  }
  let listed = false;
  $effect(() => {
    if (!listed && !tab && !kernel.settings.value.welcomeDone && kernel.connection.ready) {
      listed = true;
      void kernel.registries.loadRegistries();
    }
  });
  const statusText: Record<string, string> = {
    idle: 'Idle',
    running: 'Running',
    awaiting_approval: 'Waiting for your approval',
    limited: 'Usage limit reached',
    failed: 'Failed',
    archived: 'Archived',
  };
</script>

{#if tab && !conn}
  <div class="kk-empty">
    <p>This core was removed from this window.</p>
    <p><button type="button" class="kk-button" onclick={() => kernel.closeTab(tabIndex, target.id)}>Close the tab</button></p>
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
    <div class="kk-empty empty-hero">
      <div class="hero-copy">
        <h1>Start a chat or task</h1>
        <p class="kk-muted">Start a quick chat right away, or open a project folder to run coding tasks with an agent.</p>
        <div class="hero-actions">
          <button type="button" class="kk-button primary" disabled={!kernel.connection.ready} onclick={() => void kernel.startNewChat()}>
            <Icon name="message-plus" size={14} />
            New chat
          </button>
          <button type="button" class="kk-button" disabled={!kernel.connection.ready} onclick={() => kernel.openWorkspaceFolder()}>
            <Icon name="folder-plus" size={14} />
            Open folder…
          </button>
          <button type="button" class="kk-button" onclick={() => (kernel.dialog = { kind: 'palette' })}>
            <Icon name="command" size={14} />
            Commands
          </button>
        </div>
      </div>
      {#if !kernel.settings.value.welcomeDone && (kernel.registries.clientAvailable || kernel.registries.coreAvailable)}
        <section class="welcome" aria-labelledby="kk-welcome-title">
          <h2 id="kk-welcome-title">Welcome to Kumikit</h2>
          <p>Kumikit starts small. Agent providers and tools are plugins you choose. The starter pack lists common ones from the plugin registry.</p>
          <p class="kk-muted">Browsing it contacts {registryHosts.length > 0 ? registryHosts.join(', ') : 'the plugin registry'}. Nothing is installed unless you choose it.</p>
          <p class="welcome-actions">
            <button type="button" class="kk-button" onclick={() => kernel.dismissWelcome()}>Not now</button>
            <button type="button" class="kk-button primary" onclick={() => kernel.openStarterPack()}>Browse the starter pack</button>
          </p>
        </section>
      {/if}
    </div>
  {:else if !address || !slotProps}
    {#if conn}
      <NewThreadComposer
        {kernel}
        connection={conn}
        workspaceId={tab.workspace}
        workspaceName={wsName}
        worktreePath={tab.worktreePath}
        paneId={target.id}
      />
    {/if}
  {:else}
    <section class="pane" aria-label={thread?.title ?? 'Thread'}>
      <header class="thread-head">
        <h1 class="thread-title">{thread?.title ?? 'Thread'}</h1>
        {#if thread}
          <span class="status-pill">
            <span class="kk-status {thread.status}" aria-hidden="true"></span>
            <span>{statusText[thread.status] ?? thread.status}</span>
          </span>
          {#if isChat}
            <span class="kind-pill" title="Standalone chat (no project folder)">
              <Icon name="message" size={11} />
              <span>Chat</span>
            </span>
          {:else if thread.worktreePath}
            <span class="wt-pill" title={`Worktree: ${thread.worktreePath}`}>
              <Icon name="worktree" size={11} />
              <span>Worktree</span>
            </span>
          {:else}
            <span class="kind-pill" title={ws?.root ? `Task in ${ws.root}` : 'Project task'}>
              <Icon name="folder" size={11} />
              <span>Task</span>
            </span>
          {/if}
          <span class="model-tag">{conn?.providers.find((p) => p.id === thread.provider)?.name ?? thread.provider} · {thread.model}</span>
        {/if}
        <span class="spacer"></span>
        {#each kernel.slotsIn('thread.header', address.connection) as entry (entry)}
          <SlotHost plugin={entry.plugin} contribution={entry.contribution} props={slotProps} slots={kernel.slots} />
        {/each}
        {#if thread && (thread.status === 'running' || thread.status === 'awaiting_approval')}
          <button
            type="button"
            class="kk-icon-btn danger-btn"
            aria-label="Cancel turn"
            title="Cancel the running turn"
            onclick={() => kernel.threads.cancel(address)}
          >
            <Icon name="stop" size={12} />
          </button>
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
  .empty-hero {
    gap: 20px;
  }
  .hero-copy {
    display: grid;
    justify-items: center;
    gap: 8px;
    max-width: 440px;
  }
  .hero-copy h1 {
    margin: 0;
    font-size: var(--kk-font-size-title, x-large);
    color: var(--kk-text-primary, CanvasText);
  }
  .hero-copy p {
    margin: 0;
  }
  .hero-actions {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  .welcome {
    max-width: 500px;
    padding: 14px 18px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, Canvas);
    box-shadow: var(--kk-shadow-card, none);
    text-align: left;
  }
  .welcome h2 {
    margin: 0 0 6px;
    font-size: var(--kk-font-size-large, large);
    color: var(--kk-text-primary, CanvasText);
  }
  .welcome p {
    margin: 6px 0;
    font-size: var(--kk-font-size-small, small);
  }
  .welcome-actions {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 12px;
  }
  .banner {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 5px 12px;
    font-size: var(--kk-font-size-small, small);
    background: var(--kk-state-warning-tint, Mark);
    color: var(--kk-text-primary, MarkText);
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .pane {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .thread-head {
    display: flex;
    align-items: center;
    gap: 10px;
    height: 34px;
    padding: 0 14px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    background: var(--kk-surface-panel, Canvas);
    flex: none;
  }
  .thread-title {
    margin: 0;
    font-size: var(--kk-font-size-base, medium);
    font-weight: var(--kk-font-weight-strong, bold);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .status-pill {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    padding: 1px 7px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, Canvas);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
    white-space: nowrap;
  }
  .kind-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 7px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-card, Canvas);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
    white-space: nowrap;
  }
  .wt-pill {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 1px 7px;
    border: 1px solid var(--kk-border-subtle, CanvasText);
    border-radius: var(--kk-radius-large, 0);
    background: var(--kk-surface-raised, Canvas);
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-accent, Highlight);
    white-space: nowrap;
  }
  .model-tag {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-muted, GrayText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .spacer {
    flex: 1;
  }
  .danger-btn {
    color: var(--kk-state-danger, CanvasText);
  }
  .body {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
</style>
