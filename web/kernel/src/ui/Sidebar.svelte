<!-- Orca-inspired left sidebar: Core -> Workspace -> Thread hierarchy with collapsible
     groups, quick single-click `+` thread creation, a flat Active/Recent Threads view,
     quick filter, and docked `sidebar.section` UI plugins (06-client, Sidebar). -->
<script lang="ts">
  import { isComposing } from '@kumikit/ui-sdk';
  import type { Kernel } from '../app.svelte';
  import type { CoreConnection } from '../connections/connection.svelte';
  import { untrack } from 'svelte';
  import ContextMenu, { type ContextMenuItem } from './ContextMenu.svelte';
  import Icon from './Icon.svelte';
  import SlotHost from './SlotHost.svelte';

  let { kernel }: { kernel: Kernel } = $props();

  let filterOpen = $state(false);
  let query = $state('');
  let collapsed = $state<Record<string, boolean>>({});
  let archivedOpen = $state(false);
  type SidebarMenu =
    | {
        kind: 'thread';
        x: number;
        y: number;
        connection: string;
        workspace: string;
        threadId: string;
        title: string;
        archived: boolean;
        running: boolean;
        hasWorktree: boolean;
        isChat: boolean;
      }
    | {
        kind: 'workspace';
        x: number;
        y: number;
        connection: string;
        workspace: string;
        workspaceName: string;
        threadCount: number;
        running: boolean;
        isChat: boolean;
      };

  let menu = $state<SidebarMenu | undefined>();

  /** What a thread's menu acts on, as the row knows it. */
  type ThreadTarget = {
    connection: string;
    workspace: string;
    threadId: string;
    title: string;
    status: string;
    hasWorktree: boolean;
    isChat: boolean;
  };

  function threadTarget(
    connection: string,
    workspace: string,
    threadId: string,
    title: string,
    status: string,
    hasWorktree: boolean,
    isChat: boolean,
  ): ThreadTarget {
    return { connection, workspace, threadId, title, status, hasWorktree, isChat };
  }

  function showThreadMenu(x: number, y: number, t: ThreadTarget): void {
    menu = {
      kind: 'thread',
      x,
      y,
      connection: t.connection,
      workspace: t.workspace,
      threadId: t.threadId,
      title: t.title,
      archived: t.status === 'archived',
      running: t.status === 'running' || t.status === 'awaiting_approval',
      hasWorktree: t.hasWorktree,
      isChat: t.isChat,
    };
  }

  /** Right click: the menu opens at the pointer. */
  function openThreadMenu(e: MouseEvent, t: ThreadTarget): void {
    e.preventDefault();
    e.stopPropagation();
    showThreadMenu(e.clientX, e.clientY, t);
  }

  /** The row's `⋯` button: the menu opens under the button. */
  function openThreadMenuFromButton(e: MouseEvent, t: ThreadTarget): void {
    e.stopPropagation();
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    showThreadMenu(r.left, r.bottom, t);
  }

  function openWorkspaceMenu(
    e: MouseEvent,
    connection: string,
    workspace: string,
    workspaceName: string,
    allThreads: { status: string }[],
    isChat: boolean,
  ) {
    e.preventDefault();
    e.stopPropagation();
    menu = {
      kind: 'workspace',
      x: e.clientX,
      y: e.clientY,
      connection,
      workspace,
      workspaceName,
      threadCount: allThreads.length,
      running: allThreads.some((t) => t.status === 'running' || t.status === 'awaiting_approval'),
      isChat,
    };
  }

  function confirmDeleteThread(m: Extract<SidebarMenu, { kind: 'thread' }>) {
    const noun = m.isChat ? 'chat' : 'task';
    kernel.confirm({
      title: `Delete ${noun} “${m.title}”?`,
      message: m.hasWorktree
        ? 'This permanently deletes this task and its history, and removes its managed worktree folder if no other task uses it.'
        : `This permanently deletes this ${noun} and its message history.`,
      confirmLabel: `Delete ${noun}`,
      tone: 'danger',
      onConfirm: async () => {
        await kernel.deleteThread(m.connection, m.workspace, m.threadId);
      },
    });
  }

  function confirmDeleteWorkspace(m: Extract<SidebarMenu, { kind: 'workspace' }>) {
    if (m.isChat) {
      const message =
        m.threadCount > 0
          ? `This permanently deletes all ${m.threadCount} ${m.threadCount === 1 ? 'chat' : 'chats'} (including archived) on this core.`
          : 'This removes the Chats group from the sidebar until you start a new chat.';
      kernel.confirm({
        title: 'Delete all chats?',
        message,
        confirmLabel: 'Delete chats',
        tone: 'danger',
        onConfirm: async () => {
          await kernel.deleteWorkspace(m.connection, m.workspace);
        },
      });
      return;
    }
    const message =
      m.threadCount > 0
        ? `This permanently deletes “${m.workspaceName}” and all ${m.threadCount} ${m.threadCount === 1 ? 'task' : 'tasks'} (including archived) and any managed worktrees created for them in Kumikit. Your project files on disk will not be touched.`
        : `This removes “${m.workspaceName}” from Kumikit. Your project files on disk will not be touched.`;
    kernel.confirm({
      title: `Delete project “${m.workspaceName}”?`,
      message,
      confirmLabel: 'Delete project',
      tone: 'danger',
      onConfirm: async () => {
        await kernel.deleteWorkspace(m.connection, m.workspace);
      },
    });
  }

  const menuItems = $derived.by((): ContextMenuItem[] => {
    const m = menu;
    if (!m) return [];
    if (m.kind === 'workspace') {
      return [
        {
          id: 'delete-workspace',
          label: 'Delete',
          icon: 'trash',
          tone: 'danger',
          disabled: m.running,
          onSelect: () => confirmDeleteWorkspace(m),
        },
      ];
    }
    if (m.archived) {
      return [
        {
          id: 'unarchive',
          label: 'Unarchive',
          icon: 'archive-restore',
          onSelect: () => void kernel.unarchiveThread(m.connection, m.workspace, m.threadId),
        },
        {
          id: 'delete',
          label: 'Delete',
          icon: 'trash',
          tone: 'danger',
          onSelect: () => confirmDeleteThread(m),
        },
      ];
    }
    return [
      {
        id: 'rename',
        label: 'Rename',
        icon: 'pencil',
        disabled: m.running,
        onSelect: () =>
          kernel.renameThread({ connection: m.connection, workspace: m.workspace, thread: m.threadId }, m.title),
      },
      {
        id: 'archive',
        label: 'Archive',
        icon: 'archive',
        disabled: m.running,
        onSelect: () => void kernel.archiveThread(m.connection, m.workspace, m.threadId),
      },
      {
        id: 'delete',
        label: 'Delete',
        icon: 'trash',
        tone: 'danger',
        disabled: m.running,
        onSelect: () => confirmDeleteThread(m),
      },
    ];
  });

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
    limited: 'usage limit reached',
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
    limited: 2,
    failed: 3,
    idle: 4,
  };

  /** Flat thread list across every core and workspace for Orca-style Threads view. */
  const flatThreads = $derived.by(() => {
    const items: {
      conn: CoreConnection;
      wsId: string;
      wsName: string;
      isChat: boolean;
      id: string;
      title: string;
      status: string;
      created: string;
      model: string;
      worktreePath?: string;
    }[] = [];
    for (const conn of kernel.connections) {
      for (const ws of conn.workspaces) {
        const isChat = ws.kind === 'chat';
        for (const t of conn.threads[ws.id] ?? []) {
          if (t.status === 'archived') continue;
          const title = t.title ?? `${isChat ? 'Chat' : 'Task'} ${t.created.slice(0, 16).replace('T', ' ')}`;
          if (!matchesFilter(`${title} ${ws.name} ${conn.label}`)) continue;
          const entry: (typeof items)[number] = {
            conn,
            wsId: ws.id,
            wsName: ws.name,
            isChat,
            id: t.id,
            title,
            status: t.status,
            created: t.created,
            model: t.model,
          };
          if (t.worktreePath) entry.worktreePath = t.worktreePath;
          items.push(entry);
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

  /** All archived threads across cores/workspaces, plus filtered subset when searching. */
  const allArchivedThreads = $derived.by(() => {
    const items: {
      conn: CoreConnection;
      wsId: string;
      wsName: string;
      isChat: boolean;
      id: string;
      title: string;
      created: string;
      updated: string;
      worktreePath?: string;
    }[] = [];
    for (const conn of kernel.connections) {
      for (const ws of conn.workspaces) {
        const isChat = ws.kind === 'chat';
        for (const t of conn.threads[ws.id] ?? []) {
          if (t.status !== 'archived') continue;
          const title = t.title ?? `${isChat ? 'Chat' : 'Task'} ${t.created.slice(0, 16).replace('T', ' ')}`;
          const entry: (typeof items)[number] = {
            conn,
            wsId: ws.id,
            wsName: ws.name,
            isChat,
            id: t.id,
            title,
            created: t.created,
            updated: t.updated,
          };
          if (t.worktreePath) entry.worktreePath = t.worktreePath;
          items.push(entry);
        }
      }
    }
    return items.sort((a, b) => b.updated.localeCompare(a.updated));
  });

  const filteredArchivedThreads = $derived(
    allArchivedThreads.filter((item) => matchesFilter(`${item.title} ${item.wsName} ${item.conn.label}`)),
  );
  const showArchivedItems = $derived(
    archivedOpen || (normalizedQuery.length > 0 && filteredArchivedThreads.length > 0),
  );

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
          {@const target = threadTarget(item.conn.id, item.wsId, item.id, item.title, item.status, Boolean(item.worktreePath), item.isChat)}
          <li class="row" class:current={here}>
            <button
              type="button"
              class="thread flat-thread"
              class:current={here}
              aria-current={here ? 'page' : undefined}
              onclick={() => kernel.openTab(item.conn.id, item.wsId, item.id)}
              oncontextmenu={(e) => openThreadMenu(e, target)}
            >
              <span class="kk-status {item.status}" aria-hidden="true"></span>
              <span class="thread-main">
                <span class="title-row">
                  {#if item.isChat}
                    <span class="chat-mark" title="Chat"><Icon name="message" size={11} /></span>
                  {:else if item.worktreePath}
                    <span class="wt-mark" title={`Worktree: ${item.worktreePath}`}><Icon name="worktree" size={11} /></span>
                  {/if}
                  <span class="title">{item.title}</span>
                </span>
                <span class="meta">{item.isChat ? 'Chat' : item.wsName} · {shortTime(item.created)}</span>
              </span>
              <span class="kk-sr-only">, {statusText[item.status] ?? item.status}</span>
            </button>
            <button
              type="button"
              class="kk-icon-btn row-more"
              aria-label="Thread actions"
              aria-haspopup="menu"
              title="Thread actions"
              onclick={(e) => openThreadMenuFromButton(e, target)}
            >
              <Icon name="more" size={13} />
            </button>
          </li>
        {:else}
          <li class="kk-muted none">No active threads yet.</li>
        {/each}
      </ul>
    {:else}
      {#each kernel.connections as conn (conn.id)}
        {@const orderedWorkspaces = [
          ...conn.workspaces.filter((w) => w.kind === 'chat'),
          ...conn.workspaces.filter((w) => w.kind !== 'chat'),
        ]}
        <section class="core" aria-label={conn.label} data-core={conn.id}>
          <div class="head">
            <span
              class="core-icon"
              class:connected={conn.status === 'ready'}
              data-core-status={conn.status === 'ready' ? 'connected' : 'disconnected'}
              aria-hidden="true"
            >
              <Icon name="server" size={13} />
            </span>
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
          {#each orderedWorkspaces as ws (ws.id)}
            {@const isChat = ws.kind === 'chat'}
            {@const allWsThreads = conn.threads[ws.id] ?? []}
            {@const wsThreads = allWsThreads.filter((t) => t.status !== 'archived')}
            {@const filteredThreads = wsThreads.filter((t) => matchesFilter(`${t.title ?? ''} ${ws.name}`))}
            {@const showWs = !normalizedQuery || matchesFilter(ws.name) || filteredThreads.length > 0}
            {#if showWs}
              {@const expanded = normalizedQuery ? true : isWorkspaceOpen(conn.id, ws.id)}
              {@const current = kernel.activeTab?.connection === conn.id && kernel.activeTab?.workspace === ws.id}
              <section class="ws-group" aria-label={ws.name}>
                <div
                  class="ws-row"
                  class:current
                  role="group"
                  aria-label={ws.name}
                  oncontextmenu={(e) => openWorkspaceMenu(e, conn.id, ws.id, ws.name, allWsThreads, isChat)}
                >
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
                    title={isChat ? 'Chats (no project folder required)' : ws.root}
                    onclick={() => kernel.openTab(conn.id, ws.id)}
                  >
                    {#if isChat}
                      <span class="chat-mark" aria-hidden="true"><Icon name="message" size={12} /></span>
                    {/if}
                    <span class="ws-name">{ws.name}</span>
                    {#if wsThreads.length > 0}
                      <span class="count" aria-hidden="true">{wsThreads.length}</span>
                    {/if}
                  </button>
                  <button
                    type="button"
                    class="kk-icon-btn ws-add"
                    aria-label={isChat ? 'New chat' : `New task in ${ws.name}`}
                    title={isChat ? 'New chat' : `New task in ${ws.name}`}
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
                      {@const tLabel = t.title ?? `${isChat ? 'Chat' : 'Task'} ${t.created.slice(0, 16).replace('T', ' ')}`}
                      {@const target = threadTarget(conn.id, ws.id, t.id, tLabel, t.status, Boolean(t.worktreePath), isChat)}
                      <li class="row" class:current={here}>
                        <button
                          type="button"
                          class="thread"
                          class:current={here}
                          aria-current={here ? 'page' : undefined}
                          title={t.worktreePath
                            ? `${tLabel} (Worktree: ${t.worktreePath})`
                            : tLabel}
                          onclick={() => kernel.openTab(conn.id, ws.id, t.id)}
                          oncontextmenu={(e) => openThreadMenu(e, target)}
                        >
                          <span class="kk-status {t.status}" aria-hidden="true"></span>
                          {#if t.worktreePath}
                            <span class="wt-mark" aria-hidden="true"><Icon name="worktree" size={11} /></span>
                          {/if}
                          <span class="title">{tLabel}</span>
                          <span class="kk-sr-only">, {statusText[t.status] ?? t.status}</span>
                        </button>
                        <button
                          type="button"
                          class="kk-icon-btn row-more"
                          aria-label="Thread actions"
                          aria-haspopup="menu"
                          title="Thread actions"
                          onclick={(e) => openThreadMenuFromButton(e, target)}
                        >
                          <Icon name="more" size={13} />
                        </button>
                      </li>
                    {:else}
                      <li class="kk-muted none">{isChat ? 'No chats yet' : 'No tasks yet'}</li>
                    {/each}
                  </ul>
                {/if}
              </section>
            {/if}
          {:else}
            {#if conn.ready}<p class="kk-muted none">Open a project folder for tasks, or start a new chat.</p>{/if}
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

  {#if allArchivedThreads.length > 0}
    <section class="archived-section" aria-label="Archived threads">
      <button
        type="button"
        class="archived-toggle"
        aria-expanded={showArchivedItems}
        onclick={() => (archivedOpen = !archivedOpen)}
      >
        <Icon name={showArchivedItems ? 'chevron-down' : 'chevron-right'} size={12} />
        <Icon name="archive" size={12} />
        <span class="archived-label">Archived</span>
        <span class="count">{normalizedQuery ? filteredArchivedThreads.length : allArchivedThreads.length}</span>
      </button>
      {#if showArchivedItems}
        <ul class="archived-list">
          {#each filteredArchivedThreads as item (`${item.conn.id}/${item.wsId}/${item.id}`)}
            {@const here =
              kernel.activeTab?.connection === item.conn.id &&
              kernel.activeTab?.workspace === item.wsId &&
              kernel.activeThread?.thread === item.id}
            {@const target = threadTarget(item.conn.id, item.wsId, item.id, item.title, 'archived', Boolean(item.worktreePath), item.isChat)}
            <li class="row" class:current={here}>
              <button
                type="button"
                class="thread archived-thread"
                class:current={here}
                aria-current={here ? 'page' : undefined}
                title={`${item.title} · ${item.wsName}`}
                onclick={() => kernel.openTab(item.conn.id, item.wsId, item.id)}
                oncontextmenu={(e) => openThreadMenu(e, target)}
              >
                {#if item.isChat}
                  <span class="chat-mark" aria-hidden="true"><Icon name="message" size={11} /></span>
                {:else if item.worktreePath}
                  <span class="wt-mark" aria-hidden="true"><Icon name="worktree" size={11} /></span>
                {/if}
                <span class="title">{item.title}</span>
                <span class="ws-tag">{item.wsName}</span>
                <span class="kk-sr-only">, archived</span>
              </button>
              <button
                type="button"
                class="kk-icon-btn row-more"
                aria-label="Thread actions"
                aria-haspopup="menu"
                title="Thread actions"
                onclick={(e) => openThreadMenuFromButton(e, target)}
              >
                <Icon name="more" size={13} />
              </button>
            </li>
          {:else}
            <li class="kk-muted none">No matching archived threads</li>
          {/each}
        </ul>
      {/if}
    </section>
  {/if}
</nav>

{#if menu}
  <ContextMenu
    x={menu.x}
    y={menu.y}
    label={menu.kind === 'workspace' ? 'Project actions' : 'Thread actions'}
    items={menuItems}
    onClose={() => (menu = undefined)}
  />
{/if}

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
  .core-icon {
    display: inline-flex;
    align-items: center;
    color: var(--kk-state-danger, CanvasText);
    flex: none;
  }
  .core-icon.connected {
    color: var(--kk-state-success, Highlight);
  }
  .chat-mark {
    display: inline-flex;
    align-items: center;
    color: var(--kk-text-secondary, CanvasText);
    flex: none;
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
  .row {
    display: flex;
    align-items: center;
    gap: 2px;
    border-radius: var(--kk-radius-small, 0);
  }
  .row > .thread {
    flex: 1;
    min-width: 0;
    width: auto;
  }
  .row-more {
    width: 22px;
    min-width: 22px;
    height: 22px;
    flex: none;
    opacity: 0;
  }
  .row:hover .row-more,
  .row:focus-within .row-more,
  .row.current .row-more {
    opacity: 1;
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
  .title-row {
    display: flex;
    align-items: center;
    gap: 5px;
    min-width: 0;
  }
  .wt-mark {
    display: inline-flex;
    align-items: center;
    color: var(--kk-accent, Highlight);
    flex: none;
  }
  .archived-section {
    flex: none;
    border-top: 1px solid var(--kk-border-subtle, CanvasText);
    padding: 4px 6px 6px;
    display: flex;
    flex-direction: column;
    gap: 2px;
    max-height: 40%;
  }
  .archived-toggle {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 26px;
    padding: 0 4px;
    border: 0;
    border-radius: var(--kk-radius-small, 0);
    background: transparent;
    color: var(--kk-text-secondary, CanvasText);
    font-size: var(--kk-font-size-small, small);
    font-weight: var(--kk-font-weight-strong, bold);
    text-align: left;
    cursor: pointer;
  }
  .archived-toggle:hover {
    background: var(--kk-surface-hover, ButtonFace);
    color: var(--kk-text-primary, CanvasText);
  }
  .archived-label {
    flex: 1;
  }
  .archived-list {
    padding: 0;
    overflow: auto;
    max-height: 200px;
  }
  .archived-thread {
    color: var(--kk-text-muted, GrayText);
  }
  .ws-tag {
    margin-left: auto;
    padding: 0 5px;
    border-radius: var(--kk-radius-small, 0);
    background: var(--kk-surface-raised, Canvas);
    color: var(--kk-text-muted, GrayText);
    flex: none;
    max-width: 84px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
</style>
