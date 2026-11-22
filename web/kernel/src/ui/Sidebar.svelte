<!-- Orca-inspired left sidebar: Core -> Workspace -> Thread hierarchy with collapsible
     groups, quick single-click `+` thread creation, a flat Active/Recent Threads view,
     quick filter, and docked `sidebar.section` UI plugins (06-client, Sidebar). -->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import type { CoreConnection } from '../connections/connection.svelte';
  import { untrack } from 'svelte';
  import Icon from './Icon.svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel }: { kernel: Kernel } = $props();

  let filterOpen = $state(false);
  let query = $state('');
  let collapsed = $state<Record<string, boolean>>({});

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

  const normalizedQuery = $derived(query.trim().toLowerCase());

  function matchesFilter(text: string): boolean {
    return !normalizedQuery || text.toLowerCase().includes(normalizedQuery);
  }

  function isWorkspaceOpen(connId: string, wsId: string): boolean {
    const key = `${connId}/${wsId}`;
    if (key in collapsed) return !collapsed[key];
    return kernel.allTabs.some((t) => t.connection === connId && t.workspace === wsId);
  }

  function toggleWorkspace(connId: string, wsId: string): void {
    const key = `${connId}/${wsId}`;
    collapsed = { ...collapsed, [key]: isWorkspaceOpen(connId, wsId) };
  }

  function shortTime(iso: string): string {
    return iso.slice(5, 16).replace('T', ' ');
  }

  const statusRank: Record<string, number> = {
    awaiting_approval: 0,
    running: 1,
    failed: 2,
    idle: 3,
  };

  /** Flat thread list across every core and workspace for Orca-style Threads view. */
  const flatThreads = $derived.by(() => {
    const items: {
      conn: CoreConnection;
      wsId: string;
      wsName: string;
      id: string;
      title: string;
      status: string;
      created: string;
      model: string;
    }[] = [];
    for (const conn of kernel.connections) {
      for (const ws of conn.workspaces) {
        for (const t of conn.threads[ws.id] ?? []) {
          if (t.status === 'archived') continue;
          const title = t.title ?? `Thread ${t.created.slice(0, 16).replace('T', ' ')}`;
          if (!matchesFilter(`${title} ${ws.name} ${conn.label}`)) continue;
          items.push({
            conn,
            wsId: ws.id,
            wsName: ws.name,
            id: t.id,
            title,
            status: t.status,
            created: t.created,
            model: t.model,
          });
        }
      }
    }
    return items.sort((a, b) => {
      const ra = statusRank[a.status] ?? 9;
      const rb = statusRank[b.status] ?? 9;
      if (ra !== rb) return ra - rb;
      return b.created.localeCompare(a.created);
    });
  });

  function onFilterKey(e: KeyboardEvent) {
    if (!isComposing(e) && e.key === 'Escape') {
      query = '';
      filterOpen = false;
    }
  }
</script>

<nav class="sidebar" aria-label="Workspaces">
  <div class="bar">
    <div class="views" role="group" aria-label="Sidebar view">
      <button
        type="button"
        class="kk-icon-btn"
        class:active={kernel.leftSidebarView === 'workspaces'}
        aria-label="Workspaces tree"
        aria-pressed={kernel.leftSidebarView === 'workspaces'}
        title="Workspaces and threads tree"
        onclick={() => kernel.setLeftSidebarView('workspaces')}
      >
        <Icon name="folder" size={13} />
      </button>
      <button
        type="button"
        class="kk-icon-btn"
        class:active={kernel.leftSidebarView === 'threads'}
        aria-label="All threads"
        aria-pressed={kernel.leftSidebarView === 'threads'}
        title="Active and recent threads across workspaces"
        onclick={() => kernel.setLeftSidebarView('threads')}
      >
        <Icon name="layers" size={13} />
      </button>
    </div>
    <span class="spacer"></span>
    <button
      type="button"
      class="kk-icon-btn"
      class:active={filterOpen || query.length > 0}
      aria-label="Filter workspaces and threads"
      aria-pressed={filterOpen}
      title="Filter workspaces and threads"
      onclick={() => {
        filterOpen = !filterOpen;
        if (!filterOpen) query = '';
      }}
    >
      <Icon name="search" size={13} />
    </button>
    {#if kernel.platform.profiles.editable}
      <button
        type="button"
        class="kk-icon-btn"
        aria-label="Add a core…"
        title="Add or manage remote cores"
        onclick={() => (kernel.main = { kind: 'cores' })}
      >
        <Icon name="server" size={13} />
      </button>
    {/if}
  </div>

  {#if filterOpen}
    <div class="filter">
      <input
        class="kk-input filter-input"
        type="search"
        placeholder="Filter workspaces or threads…"
        aria-label="Filter workspaces or threads"
        bind:value={query}
        onkeydown={onFilterKey}
      />
    </div>
  {/if}

  <div class="scroll">
    {#if kernel.leftSidebarView === 'threads'}
      <ul class="flat-list">
        {#each flatThreads as item (`${item.conn.id}/${item.wsId}/${item.id}`)}
          {@const here =
            kernel.activeTab?.connection === item.conn.id &&
            kernel.activeTab?.workspace === item.wsId &&
            kernel.activeThread?.thread === item.id}
          <li>
            <button
              type="button"
              class="thread flat-thread"
              class:current={here}
              aria-current={here ? 'page' : undefined}
              onclick={() => kernel.openTab(item.conn.id, item.wsId, item.id)}
            >
              <span class="kk-status {item.status}" aria-hidden="true"></span>
              <span class="thread-main">
                <span class="title">{item.title}</span>
                <span class="meta">{item.wsName} · {shortTime(item.created)}</span>
              </span>
              <span class="kk-sr-only">, {statusText[item.status] ?? item.status}</span>
            </button>
          </li>
        {:else}
          <li class="kk-muted none">No active threads yet.</li>
        {/each}
      </ul>
    {:else}
      {#each kernel.connections as conn (conn.id)}
        <section class="core" aria-label={conn.label} data-core={conn.id}>
          <div class="head">
            <span class="kk-status {conn.status === 'ready' ? 'ready' : 'awaiting_approval'}" aria-hidden="true"></span>
            <span class="name" title={coreTitle(conn)}>{conn.label}</span>
            {#if conn.status !== 'ready'}
              <span class="state {conn.status}" role="status">{connText[conn.status] ?? conn.status}</span>
            {/if}
            <button
              type="button"
              class="kk-icon-btn"
              aria-label="Open folder…"
              title={`Open project folder on ${conn.label}`}
              onclick={() => kernel.openWorkspaceFolder(conn.id)}
              disabled={!conn.ready}
            >
              <Icon name="folder-plus" size={13} />
            </button>
          </div>
          {#each conn.workspaces as ws (ws.id)}
            {@const wsThreads = (conn.threads[ws.id] ?? []).filter((t) => t.status !== 'archived')}
            {@const filteredThreads = wsThreads.filter((t) => matchesFilter(`${t.title ?? ''} ${ws.name}`))}
            {@const showWs = !normalizedQuery || matchesFilter(ws.name) || filteredThreads.length > 0}
            {#if showWs}
              {@const expanded = normalizedQuery ? true : isWorkspaceOpen(conn.id, ws.id)}
              {@const current = kernel.activeTab?.connection === conn.id && kernel.activeTab?.workspace === ws.id}
              <section class="ws-group" aria-label={ws.name}>
                <div class="ws-row" class:current>
                  <button
                    type="button"
                    class="disclosure"
                    aria-label={expanded ? `Collapse ${ws.name}` : `Expand ${ws.name}`}
                    aria-expanded={expanded}
                    onclick={() => toggleWorkspace(conn.id, ws.id)}
                  >
                    <Icon name={expanded ? 'chevron-down' : 'chevron-right'} size={12} />
                  </button>
                  <button
                    type="button"
                    class="ws"
                    title={ws.root}
                    onclick={() => kernel.openTab(conn.id, ws.id)}
                  >
                    <span class="ws-name">{ws.name}</span>
                    {#if wsThreads.length > 0}
                      <span class="count" aria-hidden="true">{wsThreads.length}</span>
                    {/if}
                  </button>
                  <button
                    type="button"
                    class="kk-icon-btn ws-add"
                    aria-label={`New thread in ${ws.name}`}
                    title={`New thread in ${ws.name}`}
                    disabled={!conn.ready}
                    onclick={() => kernel.startDraftThread({ connection: conn.id, workspace: ws.id })}
                  >
                    <Icon name="plus" size={12} />
                  </button>
                </div>
                {#if expanded}
                  <ul>
                    {#each filteredThreads as t (t.id)}
                      {@const here = current && kernel.activeThread?.thread === t.id}
                      <li>
                        <button
                          type="button"
                          class="thread"
                          class:current={here}
                          aria-current={here ? 'page' : undefined}
                          title={t.title ?? `Thread ${shortTime(t.created)}`}
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
            {/if}
          {:else}
            {#if conn.ready}<p class="kk-muted none">Open a project folder to start.</p>{/if}
          {/each}
        </section>
      {/each}
    {/if}

    {#each kernel.slotsIn('sidebar.section') as entry (entry)}
      <section class="plugin-section" aria-label={entry.contribution.title}>
        <h2>{entry.contribution.title}</h2>
        <div class="plugin-body">
          <SlotHost plugin={entry.plugin} contribution={entry.contribution} props={sectionProps} slots={kernel.slots} />
        </div>
      </section>
    {/each}
  </div>
</nav>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--kk-surface-frame, Canvas);
    font-size: var(--kk-font-size-small, small);
  }
  .bar {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 30px;
    padding: 0 6px;
    border-bottom: 1px solid var(--kk-border-subtle, CanvasText);
    flex: none;
  }
  .views {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .spacer {
    flex: 1;
  }
  .filter {
    padding: 6px 8px 4px;
    flex: none;
  }
  .filter-input {
    padding: 4px 8px;
    font-size: var(--kk-font-size-small, small);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 6px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .core {
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .core + .core {
    padding-top: 8px;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
  }
  .head {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 2px 4px;
    color: var(--kk-text-secondary, CanvasText);
  }
  .name {
    flex: 1;
    font-weight: var(--kk-font-weight-strong, bold);
    letter-spacing: 0.02em;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .state {
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-state-warning, CanvasText);
  }
  .ws-group {
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .ws-row {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: var(--kk-radius-small, 0);
  }
  .ws-row:hover {
    background: var(--kk-surface-hover, Canvas);
  }
  .ws-row.current {
    background: var(--kk-surface-selected, Highlight);
  }
  .disclosure {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    width: 20px;
    height: 24px;
    padding: 0;
    border: 0;
    background: transparent;
    color: var(--kk-text-muted, GrayText);
    cursor: pointer;
    flex: none;
  }
  .disclosure:hover {
    color: var(--kk-text-primary, CanvasText);
  }
  .ws {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: 1;
    min-width: 0;
    height: 26px;
    padding: 0 4px 0 0;
    border: 0;
    background: transparent;
    text-align: left;
    font-weight: var(--kk-font-weight-strong, bold);
    color: var(--kk-text-primary, CanvasText);
    cursor: pointer;
  }
  .ws-name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    flex: 1;
  }
  .count {
    color: var(--kk-text-muted, GrayText);
    font-weight: var(--kk-font-weight-normal, normal);
  }
  .ws-add {
    width: 22px;
    min-width: 22px;
    height: 22px;
    opacity: 0;
  }
  .ws-row:hover .ws-add,
  .ws-row:focus-within .ws-add,
  .ws-row.current .ws-add {
    opacity: 1;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0 0 0 14px;
    display: flex;
    flex-direction: column;
    gap: 1px;
  }
  .flat-list {
    padding: 0;
    gap: 2px;
  }
  .thread {
    display: flex;
    align-items: center;
    gap: 7px;
    width: 100%;
    padding: 4px 8px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    text-align: left;
    color: var(--kk-text-secondary, CanvasText);
    cursor: pointer;
  }
  .thread:hover {
    background: var(--kk-surface-hover, Canvas);
    color: var(--kk-text-primary, CanvasText);
  }
  .thread.current {
    background: var(--kk-surface-selected, Highlight);
    color: var(--kk-text-primary, CanvasText);
  }
  .flat-thread {
    align-items: flex-start;
    padding: 6px 8px;
  }
  .flat-thread .kk-status {
    margin-top: 5px;
  }
  .thread-main {
    display: flex;
    flex-direction: column;
    min-width: 0;
    flex: 1;
  }
  .title {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .meta {
    color: var(--kk-text-muted, GrayText);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .none {
    padding: 4px 8px;
    margin: 0;
  }
  .plugin-section {
    padding-top: 8px;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .plugin-section h2 {
    margin: 0 4px;
    font-size: var(--kk-font-size-small, small);
    color: var(--kk-text-secondary, CanvasText);
  }
  .plugin-body {
    min-height: 160px;
    display: flex;
    flex-direction: column;
  }
</style>
